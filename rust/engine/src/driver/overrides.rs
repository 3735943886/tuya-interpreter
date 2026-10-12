//! Override blocks: validated, applied to the device before classification and to the descriptor after.
use crate::DpKind;
use crate::{adapter::*, assembly::*, data::*, plan::Plan};
use serde_json::{Value as V, json};
use std::sync::Arc;

pub(super) fn prepare(
    device: &V,
    options: &crate::DriverOptions,
    rules: &V,
) -> Result<(V, V, V, V), String> {
    let mut d = device.clone();
    ensure_tables(&mut d);
    let mut dpmap = json!({});
    let quirk = if !options.use_quirks {
        &V::Null
    } else {
        &rules["quirks"][s(&d["product_id"])]
    };
    apply_quirk(&mut d, &mut dpmap, quirk);
    map_initial_status(&mut d["status"], quirk);
    let info = device_info(&d, quirk, rules);
    merge(&mut dpmap, &options.dpmap);
    let mut original = dpmap.clone();
    for (id, meta) in obj(&d["local_strategy"]) {
        let code = strategy_code(&meta);
        if !code.is_empty() {
            original[&id] = json!(code);
        }
    }
    validate_mapping(&options.overrides, rules)?;
    let mut overrides = rules["overrides"].clone();
    if !overrides.is_object() {
        overrides = json!({});
    }
    merge(&mut overrides, &options.overrides);
    let mut block = json!({});
    for key in [s(&d["product_id"]), s(&d["id"])] {
        let b = &overrides[key];
        if !b.is_null() {
            validate(b, rules, false)?;
            merge(&mut block, b);
        }
    }
    validate(&block, rules, true)?;
    if truth(&block["category"]) {
        d["category"] = block["category"].clone();
    }
    for (id, patch) in obj(&block["dp"]) {
        let code = s(&patch["code"]);
        if patch.get("type").is_none() {
            let old = s(&original[&id]);
            if old.is_empty() {
                return Err(format!("dp.{id}: no dp {id} to rename"));
            }
            for table in ["function", "status_range", "status"] {
                if let Some(v) = remove(&mut d[table], old) {
                    d[table][code] = v;
                }
            }
            for v in dpmap
                .as_object_mut()
                .into_iter()
                .flat_map(|m| m.values_mut())
            {
                if *v == old {
                    *v = json!(code);
                }
            }
        } else {
            let op = json!({"op":"DefineDp","dpid":id,"code":code,"type":patch["type"],"mode":patch.get("mode").cloned().unwrap_or(json!("RW")),"values":patch.get("values").cloned().unwrap_or(json!({})),"report_type":patch["report_type"]});
            patch_op(&mut d, &mut dpmap, &op);
        }
    }
    for (code, remap) in obj(&block["remap"]) {
        let alias = &remap["alias"];
        for table in ["function", "status_range"] {
            if normalized(s(&d[table][&code]["type"])) == "Enum" {
                let mut spec = parse(&d[table][&code]["values"]);
                spec["range"] = json!(
                    items(&spec["range"])
                        .iter()
                        .map(|v| alias.get(s(v)).cloned().unwrap_or_else(|| v.clone()))
                        .collect::<Vec<_>>()
                );
                d[table][&code]["values"] = spec;
            }
        }
        if d["status"][&code].is_string() {
            let raw = s(&d["status"][&code]);
            if let Some(v) = alias.get(raw) {
                d["status"][&code] = v.clone();
            }
        }
    }
    for code in codes(&block["remove"]) {
        for table in ["function", "status_range", "status"] {
            remove(&mut d[table], &code);
        }
        if let Some(map) = dpmap.as_object_mut() {
            map.retain(|_, v| *v != code);
        }
    }
    Ok((d, block, dpmap, info))
}
fn unknown(v: &V, allowed: &[&str]) -> Result<(), String> {
    if let Some(o) = v.as_object() {
        for k in o.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(format!("unknown key {k}"));
            }
        }
    } else if !v.is_null() {
        return Err("must be object".into());
    }
    Ok(())
}
/// The converters an override block can turn on.
const CONVERTERS: [&str; 1] = ["declarative"];

fn validate(b: &V, rules: &V, complete: bool) -> Result<(), String> {
    unknown(
        b,
        &[
            "dp",
            "remove",
            "category",
            "props",
            "device",
            "converters",
            "remap",
            "expose_unused",
            "auto",
            "cover",
            "delta",
        ],
    )?;
    for key in ["dp", "props", "remap", "converters"] {
        if b.get(key).is_some() && !b[key].is_object() {
            return Err(format!("{key}: must be object"));
        }
    }
    if b.get("category").is_some() && b["category"].as_str().is_none_or(str::is_empty) {
        return Err("category: needs a nonempty string".into());
    }
    for (key, value) in fields(&b["device"]) {
        if !value.is_null() && !value.is_string() {
            return Err(format!("device.{key}: needs a string or null"));
        }
    }
    for (_, d) in obj(&b["dp"]) {
        unknown(&d, &["code", "type", "values", "mode", "report_type"])?;
        if !truth(&d["code"]) || d.get("type").is_some() && normalized(s(&d["type"])).is_empty() {
            return Err("invalid dp definition".into());
        }
        if d.get("type").is_none() && obj(&d).len() != 1 {
            return Err("a rename takes only a `code`".into());
        }
        if d.get("mode").is_some() && !["R", "W", "RW"].contains(&s(&d["mode"])) {
            return Err("invalid mode".into());
        }
    }
    for k in ["auto", "expose_unused"] {
        if b.get(k).is_some() && !b[k].is_boolean() {
            return Err(format!("{k}: true or false"));
        }
    }
    if b.get("remove").is_some()
        && (!b["remove"].is_array() || items(&b["remove"]).iter().any(|v| !v.is_string()))
    {
        return Err("remove needs list of codes".into());
    }
    unknown(&b["device"], &["kind", "class", "label", "vendor", "model"])?;
    for (_, p) in obj(&b["props"]) {
        let src = p.get("src").is_some();
        unknown(
            &p,
            if src {
                &[
                    "src", "type", "label", "class", "category", "unit", "series", "rw", "role",
                    "min", "max", "step", "options", "send",
                ]
            } else {
                &[
                    "label", "class", "category", "unit", "series", "rw", "role", "hide",
                ]
            },
        )?;
        if !p.is_object() {
            return Err("property patch must be object".into());
        }
        validate_property(&p, src, false)?;
        if p.get("hide").is_some() && !p["hide"].is_boolean() {
            return Err("hide needs bool".into());
        }
        if !p["role"].is_null() && !p["role"].is_string() {
            return Err("invalid role".into());
        }
        if p.get("rw").is_some() && (!p["rw"].is_boolean() || !src && p["rw"] != false) {
            return Err("rw can only be false".into());
        }
        if src {
            if !p["src"].is_string() || !truth(&p["src"]) {
                return Err("src needs code".into());
            }
            if p.get("type").is_some()
                && !["binary", "number", "select", "text", "trigger"].contains(&s(&p["type"]))
            {
                return Err("invalid property type".into());
            }
            if (p["type"] == "trigger") != p.get("send").is_some() {
                return Err("trigger needs send".into());
            }
        }
    }
    for (_, r) in obj(&b["remap"]) {
        unknown(&r, &["alias", "invert"])?;
        if !r.is_object() {
            return Err("remap needs object".into());
        }
        if r.get("invert").is_some() && !r["invert"].is_boolean() {
            return Err("invert needs bool".into());
        }
        if !r["alias"].is_null() && !r["alias"].is_object() {
            return Err("alias needs object".into());
        }
        let values: Vec<_> = obj(&r["alias"]).values().cloned().collect();
        for (i, v) in values.iter().enumerate() {
            if !v.is_string() {
                return Err("alias values need strings".into());
            }
            if values[..i].contains(v) {
                return Err("duplicate alias".into());
            }
        }
    }
    fn cover(c: &V, rules: &V, nested: bool) -> Result<(), String> {
        if !c.is_object() {
            return Err("cover needs object".into());
        }
        for (k, v) in obj(c) {
            if k == "settle" {
                if !v.is_number() || n(&v) < 0.0 {
                    return Err("invalid settle".into());
                }
            } else if k == "state_source" {
                if !["control", "inferred", "none"].contains(&s(&v)) {
                    return Err("invalid state_source".into());
                }
            } else if rules["cover_defaults"].get(&k).is_some() {
                if !v.is_boolean() {
                    return Err("cover needs bool".into());
                }
            } else if nested && v.is_object() {
                cover(&v, rules, false)?
            } else {
                return Err(format!("unknown cover key {k}"));
            }
        }
        Ok(())
    }
    if b.get("cover").is_some() {
        cover(&b["cover"], rules, true)?
    }
    for name in obj(&b["converters"]).keys() {
        if !CONVERTERS.contains(&name.as_str()) {
            return Err(format!("unknown converter {name}"));
        }
    }
    if let Some(config) = b["converters"].get("declarative") {
        validate_converter(config, complete)?;
    }
    if b.get("delta").is_some() {
        unknown(&b["delta"], &["accept_passive"])?;
        if b["delta"].get("accept_passive").is_some() && !b["delta"]["accept_passive"].is_boolean()
        {
            return Err("delta needs bool".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_mapping(mapping: &V, rules: &V) -> Result<(), String> {
    if !mapping.is_object() {
        return Err("overrides must be object".into());
    }
    for (name, block) in fields(mapping) {
        if !block.is_object() {
            return Err(format!("{name}: override block must be object"));
        }
        validate(block, rules, false).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

fn validate_property(p: &V, definition: bool, converter: bool) -> Result<(), String> {
    for key in ["label", "class", "category", "unit", "series", "role"] {
        if p.get(key).is_some() && !p[key].is_null() && !p[key].is_string() {
            return Err(format!("property {key}: needs string or null"));
        }
    }
    if !p["category"].is_null() && !["diagnostic", "config"].contains(&s(&p["category"])) {
        return Err("invalid category".into());
    }
    if !p["series"].is_null() && !["counter", "gauge"].contains(&s(&p["series"])) {
        return Err("invalid series".into());
    }
    if p.get("rw").is_some() && !p["rw"].is_boolean() {
        return Err("rw needs bool".into());
    }
    if definition {
        if (converter || p.get("type").is_some())
            && !["binary", "number", "select", "text", "trigger", "event"].contains(&s(&p["type"]))
        {
            return Err("invalid property type".into());
        }
        for key in ["min", "max", "step"] {
            if p.get(key).is_some() && !p[key].is_number() {
                return Err(format!("{key} needs number"));
            }
        }
        if p.get("step").is_some() && n(&p["step"]) <= 0.0 {
            return Err("step must be positive".into());
        }
        if p.get("min").is_some() && p.get("max").is_some() && n(&p["min"]) > n(&p["max"]) {
            return Err("min exceeds max".into());
        }
        if let Some(options) = p.get("options") {
            if !options.is_array() || items(options).iter().any(|v| !v.is_string()) {
                return Err("options need array of strings".into());
            }
            if items(options)
                .iter()
                .enumerate()
                .any(|(i, v)| items(options)[..i].contains(v))
            {
                return Err("duplicate options".into());
            }
        }
        if converter && ["select", "event"].contains(&s(&p["type"])) && p.get("options").is_none() {
            return Err("select/event needs options".into());
        }
    }
    Ok(())
}

pub(crate) fn value_fits(value: &V, definition: &V) -> bool {
    match s(&definition["type"]) {
        "binary" => value.is_boolean(),
        "number" => value.is_number(),
        "text" => value.is_string(),
        "select" => items(&definition["options"]).contains(value),
        _ => false,
    }
}

fn validate_converter(config: &V, complete: bool) -> Result<(), String> {
    if !config.is_object()
        || (config.get("props").is_some() || complete) && !config["props"].is_object()
    {
        return Err("declarative converter and props must be objects".into());
    }
    if config.get("write").is_some() && !config["write"].is_object() {
        return Err("write must be object".into());
    }
    for (name, definition) in fields(&config["props"]) {
        if name.is_empty() || name == "available" || !definition.is_object() {
            return Err("invalid converter property".into());
        }
        unknown(
            definition,
            &[
                "type", "label", "class", "category", "unit", "series", "rw", "role", "min", "max",
                "step", "options", "requires", "group", "src",
            ],
        )?;
        validate_property(definition, true, complete)?;
        if complete
            && (truth(&definition["rw"]) || definition["type"] == "trigger")
            && config["write"].get(name).is_none()
        {
            return Err("writable declarative property needs write program".into());
        }
    }
    for method in ["update", "timer", "reset"] {
        if let Some(program) = config.get(method) {
            validate_program(program, &config["props"], 0, complete, method != "reset")?;
        }
    }
    for (prop, program) in fields(&config["write"]) {
        if complete && config["props"].get(prop).is_none() {
            return Err(format!("write: unknown property {prop}"));
        }
        validate_program(program, &config["props"], 0, complete, true)?;
    }
    Ok(())
}

fn validate_program(
    program: &V,
    props: &V,
    depth: usize,
    complete: bool,
    effects: bool,
) -> Result<(), String> {
    if depth >= 64 {
        return Err("rule nesting limit exceeded".into());
    }
    if !program.is_array() {
        return Err("rule program must be an array".into());
    }
    for stmt in items(program) {
        if !stmt.is_object() {
            return Err("statement must be object".into());
        }
        if !effects
            && ["emit", "timer", "send"]
                .iter()
                .any(|key| stmt.get(key).is_some())
        {
            return Err("reset programs can only assign state".into());
        }
        if stmt.get("if").is_some() {
            unknown(stmt, &["if", "then", "else"])?;
            for branch in ["then", "else"] {
                if let Some(program) = stmt.get(branch) {
                    validate_program(program, props, depth + 1, complete, effects)?;
                }
            }
        } else if let Some(path) = stmt.get("set") {
            unknown(stmt, &["set", "value"])?;
            let parts = crate::expr::target(path);
            if parts.first().is_none_or(|p| p != "state") {
                return Err("rules can assign only state slots".into());
            }
        } else if let Some(prop) = stmt.get("emit") {
            unknown(stmt, &["emit", "value"])?;
            if prop.as_str().is_none_or(str::is_empty) || complete && props.get(s(prop)).is_none() {
                return Err("emit: unknown property".into());
            }
        } else if let Some(name) = stmt.get("timer") {
            unknown(stmt, &["timer", "after"])?;
            if name.as_str().is_none_or(str::is_empty) {
                return Err("timer needs name".into());
            }
            if let Some(after) = stmt.get("after")
                && !after.is_object()
                && !after.is_null()
                && (!after.is_number() || n(after) < 0.0)
            {
                return Err("invalid timer delay".into());
            }
        } else if let Some(send) = stmt.get("send") {
            unknown(stmt, &["send"])?;
            unknown(send, &["code", "value"])?;
            if !send.is_object() || send.get("code").is_none() || send.get("value").is_none() {
                return Err("send needs code and value".into());
            }
        } else if let Some(name) = stmt.get("let") {
            unknown(stmt, &["let", "value"])?;
            if name.as_str().is_none_or(str::is_empty) {
                return Err("let needs name".into());
            }
        } else {
            return Err("unknown rule statement".into());
        }
    }
    Ok(())
}
pub(super) fn patch_descriptor(
    a: &mut Assembly,
    plans: &mut Vec<Plan>,
    block: &V,
    device: &V,
    rules: &V,
) -> Result<(), String> {
    for (prop, p) in obj(&block["props"]) {
        if prop == "available" {
            return Err("props.available cannot be overridden".into());
        }
        if p.get("src").is_some() {
            let code = s(&p["src"]);
            let e = device["status_range"]
                .get(code)
                .or_else(|| device["function"].get(code))
                .ok_or_else(|| format!("props.{prop}.src: no dp {code}"))?;
            let kind = normalized(s(&e["type"]));
            let natural = match kind {
                "Boolean" => "binary",
                "Integer" => "number",
                "Enum" => "select",
                _ => "text",
            };
            let ty = p.get("type").map(s).unwrap_or(natural);
            if ty != "trigger" && ty != "text" && ty != natural {
                return Err("property type does not fit dp".into());
            }
            let writable = device["function"].get(code).is_some();
            let rw = ty != "trigger" && ty != "text" && p.get("rw").map(truth).unwrap_or(writable);
            if (rw || ty == "trigger") && !writable {
                return Err("dp is not writable".into());
            }
            let role = Role {
                code: code.into(),
                kind: DpKind::parse(kind).ok_or("invalid dp kind")?,
                spec: parse(&e["values"]),
                report_type: V::Null,
            };
            let mut def = json!({"type":ty,"src":code});
            if rw {
                def["rw"] = json!(true)
            }
            if ty == "number" {
                for (k, raw) in [
                    ("min", role.lo()),
                    ("max", role.hi()),
                    ("step", n(&role.spec["step"])),
                ] {
                    def[k] = p
                        .get(k)
                        .cloned()
                        .unwrap_or_else(|| number(raw / role.scale()));
                }
                let unit = p.get("unit").unwrap_or(&role.spec["unit"]);
                if truth(unit) {
                    def["unit"] = unit.clone();
                }
            }
            if ty == "select" {
                def["options"] = p
                    .get("options")
                    .filter(|v| truth(v))
                    .cloned()
                    .unwrap_or_else(|| json!(role.range()));
            }
            for k in ["role", "label", "class", "category", "series"] {
                if !p[k].is_null() {
                    def[k] = p[k].clone();
                }
            }
            let idx = plans.len();
            plans.push(Plan {
                platform: "override".into(),
                key: code.into(),
                identity: json!({}),
                roles: Roles::new(),
                depends_on: vec![code.into()],
                slot_kind: None,
                update_all: false,
                config: json!({}),
            });
            Arc::make_mut(&mut a.descriptor)["props"][&prop] = def;
            let trigger = ty == "trigger";
            a.bindings.insert(
                prop,
                Binding::new(
                    Source::Dp {
                        plan: idx,
                        role: Box::new(role),
                        trigger,
                        send: p["send"].clone(),
                    },
                    trigger || rw,
                ),
            );
            continue;
        }
        let Some(def) = Arc::make_mut(&mut a.descriptor)["props"].get_mut(&prop) else {
            return Err(format!("props.{prop}: no such property"));
        };
        if truth(&p["hide"]) {
            remove(&mut Arc::make_mut(&mut a.descriptor)["props"], &prop);
            a.bindings.shift_remove(&prop);
            continue;
        }
        for k in ["label", "class", "category", "unit", "series", "role"] {
            if let Some(v) = p.get(k) {
                if v.is_null() {
                    remove(def, k);
                } else if (k == "unit" || k == "series") && def["type"] != "number" {
                    return Err(format!("{k} only applies to number"));
                } else {
                    def[k] = v.clone();
                }
            }
        }
        if p["rw"] == false {
            if def["type"] == "trigger" || def["type"] == "event" {
                return Err("trigger/event cannot be made read only".into());
            }
            remove(def, "rw");
            if items(&rules["rw_roles"]).contains(&def["role"]) {
                remove(def, "role");
            }
            if let Some(b) = a.bindings.get_mut(&prop) {
                b.writable = false;
            }
        }
    }
    for (k, v) in obj(&block["device"]) {
        if v.is_null() {
            remove(Arc::make_mut(&mut a.descriptor), &k);
        } else {
            Arc::make_mut(&mut a.descriptor)[&k] = v;
        }
    }
    let used: Vec<_> = obj(&a.descriptor["props"])
        .values()
        .map(|d| d["group"].clone())
        .collect();
    if let Some(groups) = Arc::make_mut(&mut a.descriptor)
        .get_mut("groups")
        .and_then(V::as_object_mut)
    {
        groups.retain(|k, _| used.contains(&json!(k)));
        if groups.is_empty() {
            remove(Arc::make_mut(&mut a.descriptor), "groups");
        }
    }
    Ok(())
}
