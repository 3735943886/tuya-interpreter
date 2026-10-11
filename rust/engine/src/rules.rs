//! Rule-pack layering and validation over JSON the host decoded; the engine reads no files.
use crate::data::{items, s};
use serde_json::{Value, json};
use std::sync::OnceLock;

fn schema() -> Result<&'static Value, String> {
    static SCHEMA: OnceLock<Result<Value, String>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            serde_json::from_str(include_str!("../schema.json"))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// A member directive: `{"$delete": true}` or `{"$append": [...]}`.
fn directive<'v>(value: &'v Value, name: &str) -> Option<&'v Value> {
    value
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get(name))
}

/// Merge a rule layer: objects merge recursively, other values replace, `$delete` removes a member
/// and `$append` extends an array.
fn patch(dst: &mut Value, src: &Value) {
    let Some(fields) = src.as_object() else {
        *dst = src.clone();
        return;
    };
    if !dst.is_object() {
        *dst = json!({});
    }
    let target = dst.as_object_mut().expect("initialized object");
    for (key, value) in fields {
        if directive(value, "$delete") == Some(&Value::Bool(true)) {
            target.remove(key);
        } else if let Some(extra) = directive(value, "$append").and_then(Value::as_array) {
            match target.get_mut(key) {
                Some(Value::Array(items)) => items.extend(extra.iter().cloned()),
                _ => {
                    target.insert(key.clone(), Value::Array(extra.clone()));
                }
            }
        } else {
            patch(target.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

// Interpret the bundled schema's vocabulary, so validation stays language-neutral.
fn check(value: &Value, schema: &Value, root: &Value, at: &str) -> Result<(), String> {
    if let Some(reference) = schema["$ref"].as_str() {
        return check(
            value,
            root.pointer(reference.trim_start_matches('#'))
                .ok_or("invalid schema reference")?,
            root,
            at,
        );
    }
    let valid = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("number") => value.is_number(),
        Some("boolean") => value.is_boolean(),
        None => true,
        _ => false,
    };
    if !valid {
        return Err(format!("rules {at}: expected {}", schema["type"]));
    }
    if let Some(expected) = schema.get("const")
        && value != expected
    {
        return Err(format!("rules {at}: expected {expected}"));
    }
    if let Some(choices) = schema["enum"].as_array()
        && !choices.contains(value)
    {
        return Err(format!("rules {at}: unsupported value {value}"));
    }
    if let Some(text) = value.as_str()
        && text.chars().count() < schema["minLength"].as_u64().unwrap_or(0) as usize
    {
        return Err(format!("rules {at}: string too short"));
    }
    if let Some(fields) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required {
                if !fields.contains_key(key.as_str().ok_or("invalid schema required field")?) {
                    return Err(format!("rules {at}: missing {key}"));
                }
            }
        }
        for (key, item) in fields {
            let child = schema["properties"]
                .get(key)
                .or_else(|| schema.get("additionalProperties").filter(|v| v.is_object()));
            if let Some(child) = child {
                check(item, child, root, &format!("{at}/{key}"))?;
            } else if schema["additionalProperties"] == false {
                return Err(format!("rules {at}: unknown field {key}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        for (i, item) in items.iter().enumerate() {
            if let Some(child) = schema.get("items") {
                check(item, child, root, &format!("{at}/{i}"))?;
            }
            if schema["uniqueItems"] == true && items[..i].contains(item) {
                return Err(format!("rules {at}: duplicate item"));
            }
        }
    }
    Ok(())
}

pub fn validate(value: &Value) -> Result<(), String> {
    let schema = schema()?;
    check(value, schema, schema, "")
}

/// One rule layer: a rule object, or `{"source": name, "rules": object}` to name it in reports.
fn layer(v: &Value, index: usize) -> (String, &Value) {
    match (v.get("source").and_then(Value::as_str), v.get("rules")) {
        (Some(source), Some(rules)) if v.as_object().is_some_and(|o| o.len() == 2) => {
            (source.to_string(), rules)
        }
        _ => (format!("layer {index}"), v),
    }
}

/// Merge `layers` in order over `base` (decoded by the host: the engine reads no files) and
/// validate the result. With `tolerant`, a layer that would make the rules invalid is reported in
/// `warnings` and left out. Returns `{rules, sources, warnings}`.
pub fn compose(input: &Value) -> Result<Value, String> {
    let base = input.get("base").cloned().unwrap_or_else(|| json!({}));
    if !base.is_object() {
        return Err("rule base must be a JSON object".into());
    }
    let tolerant = input["tolerant"] == true;
    let schema = schema()?;
    let mut partial = schema.clone();
    partial
        .as_object_mut()
        .ok_or("invalid schema")?
        .remove("required");
    let mut result = base;
    let mut sources = vec![];
    let mut warnings = vec![];
    for (index, raw) in items(&input["layers"]).iter().enumerate() {
        let (source, value) = layer(raw, index);
        let merged = (|| {
            if !value.is_object() {
                return Err("rule layer must be a JSON object".to_string());
            }
            let mut next = result.clone();
            patch(&mut next, value);
            if tolerant {
                let complete = schema["required"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().all(|k| result.get(s(k)).is_some()));
                check(&next, if complete { schema } else { &partial }, schema, "")?;
            }
            Ok(next)
        })();
        match merged {
            Ok(next) => {
                result = next;
                sources.push(json!(source));
            }
            Err(error) if tolerant => warnings.push(json!(format!("{source}: {error}"))),
            Err(error) => return Err(format!("{source}: {error}")),
        }
    }
    check(&result, schema, schema, "")?;
    Ok(json!({"rules":result,"sources":sources,"warnings":warnings}))
}
