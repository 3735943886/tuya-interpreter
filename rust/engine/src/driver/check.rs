//! il.md section 5: a write checked against the descriptor before anything is sent.
use crate::data::*;
use serde_json::{Value as V, json};

pub fn check(desc: &V, state: &V, prop: &str, value: &V) -> Result<V, (String, String)> {
    let error = |code: &str, reason: String| Err((code.into(), reason));
    let Some(p) = desc["props"].get(prop) else {
        return error("unknown_property", prop.into());
    };
    let t = s(&p["type"]);
    if t == "event" || !truth(&p["rw"]) && t != "trigger" {
        return error("read_only", prop.into());
    }
    if let Some(req) = p.get("requires") {
        let ok = if req.is_string() {
            state[s(req)] == true
        } else {
            items(&req["in"])
                .iter()
                .any(|v| eq(v, &state[s(&req["prop"])]))
        };
        if !ok {
            return error("requires_unmet", format!("{prop} requires {req}"));
        }
    }
    match t {
        "trigger" => Ok(V::Null),
        "binary" => {
            if value.is_boolean() {
                return Ok(value.clone());
            }
            let key = if value.is_string() {
                s(value).to_lowercase()
            } else if value.is_i64() {
                value.to_string()
            } else {
                String::new()
            };
            match key.as_str() {
                "on" | "true" | "1" => Ok(json!(true)),
                "off" | "false" | "0" => Ok(json!(false)),
                _ => error("invalid_value", format!("not a boolean: {value}")),
            }
        }
        "number" => {
            if !value.is_number() {
                return error("invalid_value", format!("not a finite number: {value}"));
            }
            let v = n(value);
            let lo = &p["min"];
            let hi = &p["max"];
            if (!lo.is_null() && v < n(lo)) || (!hi.is_null() && v > n(hi)) {
                return error("out_of_range", format!("{lo}..{hi}"));
            }
            let step = n(&p["step"]);
            if step != 0.0 {
                let q = (v - n(lo)) / step;
                if (q - q.round_ties_even()).abs() * step.abs() > 1e-9 {
                    return error("bad_step", format!("step {step} from {}", n(lo)));
                }
            }
            Ok(value.clone())
        }
        "select" => {
            if items(&p["options"]).contains(value) {
                Ok(value.clone())
            } else {
                error("invalid_value", format!("not one of {}", p["options"]))
            }
        }
        "text" => {
            if value.is_string() && !s(value).is_empty() {
                Ok(value.clone())
            } else {
                error("invalid_value", "non-empty string required".into())
            }
        }
        _ => error("unsupported", format!("type {t}")),
    }
}
