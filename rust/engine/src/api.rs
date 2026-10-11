use crate::{adapter::*, data::*, driver::Driver, driver::check, entity, plan::Plan};
use serde::de::DeserializeOwned;
use serde_json::{Value as V, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
struct Pack {
    rules: V,
    fingerprint: String,
}
static PACKS: OnceLock<Mutex<HashMap<String, Arc<Pack>>>> = OnceLock::new();
fn fingerprint(rules: &V) -> String {
    format!("{:x}", Sha256::digest(rules.to_string().as_bytes()))
}
fn ordered_eq(left: &V, right: &V) -> bool {
    match (left, right) {
        (V::Object(left), V::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|((lk, lv), (rk, rv))| lk == rk && ordered_eq(lv, rv))
        }
        (V::Array(left), V::Array(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(lv, rv)| ordered_eq(lv, rv))
        }
        _ => left == right,
    }
}
fn decode<T: DeserializeOwned>(input: &V, field: &str) -> Result<T, String> {
    serde_json::from_value(input[field].clone()).map_err(|e| format!("{field}: {e}"))
}
/// The request's explicit quirk, else the rule pack's quirk for the device's product.
fn quirk<'a>(input: &'a V, device: &V, rules: &'a V) -> &'a V {
    input
        .get("quirk")
        .filter(|v| !v.is_null())
        .unwrap_or(&rules["quirks"][s(&device["product_id"])])
}
pub fn request(input: &V) -> Result<V, String> {
    if s(&input["op"]) == "merge_rules" {
        return crate::rules::compose(input);
    }
    let registry = PACKS.get_or_init(|| Mutex::new(HashMap::new()));
    if input["op"] == "install_rules" {
        let rules = input["rules"].clone();
        if rules["version"] != 1 {
            return Err("unsupported rule version".into());
        }
        let id = input["rules_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("rules_id must be a nonempty string")?
            .to_string();
        let mut packs = registry.lock().map_err(|_| "rule registry poisoned")?;
        if let Some(old) = packs.get(&id) {
            if !ordered_eq(&old.rules, &rules) {
                return Err("rule id already registered with different content".into());
            }
            return Ok(json!(true));
        }
        crate::rules::validate(&rules)?;
        let fingerprint = fingerprint(&rules);
        packs.insert(id, Arc::new(Pack { rules, fingerprint }));
        return Ok(json!(true));
    }
    let needs_rules = matches!(
        s(&input["op"]),
        "create"
            | "handle"
            | "on_update"
            | "unused"
            | "quirk_schema"
            | "device_info"
            | "classify"
            | "read"
            | "write"
            | "adapter_create"
            | "adapter_read"
            | "strategy_read"
    );
    let cached = if needs_rules && input["rules"].is_null() {
        registry
            .lock()
            .map_err(|_| "rule registry poisoned")?
            .get(s(&input["rules_id"]))
            .cloned()
    } else {
        None
    };
    let rules = cached
        .as_ref()
        .map(|pack| &pack.rules)
        .unwrap_or(&input["rules"]);
    let inline_fingerprint;
    let pack_fingerprint = if needs_rules {
        if let Some(pack) = &cached {
            pack.fingerprint.as_str()
        } else {
            if rules.is_null() {
                return Err("rules are required or rules_id is not registered".into());
            }
            if rules["version"] != 1 {
                return Err("unsupported rule version".into());
            }
            crate::rules::validate(rules)?;
            inline_fingerprint = fingerprint(rules);
            &inline_fingerprint
        }
    } else {
        ""
    };
    match s(&input["op"]) {
        "create" => {
            let mut d = Driver::new(&input["device"], &input["options"], rules)?;
            if cached.is_some() {
                d.rules_id = s(&input["rules_id"]).into();
            }
            d.rules_fingerprint = pack_fingerprint.into();
            let mut warnings = vec![];
            for (code, r) in obj(&d.block["remap"]) {
                if truth(&r["invert"])
                    && d.covers.iter().any(|c| {
                        [&c.cur, &c.setp, &c.ins]
                            .iter()
                            .any(|r| r.as_ref().is_some_and(|r| r.code == code))
                    })
                {
                    warnings.push(format!("remap.{code}.invert on a cover is deprecated: move it to the cover settings"));
                }
            }
            if d.block["converters"].get("cover_motion").is_some() {
                warnings.push(
                    "converters.cover_motion is deprecated: move it to the cover settings".into(),
                );
            }
            Ok(json!({"driver":d,"warnings":warnings}))
        }
        "handle" => {
            let mut d: Driver = decode(input, "driver")?;
            if d.rules_fingerprint != pack_fingerprint {
                return Err("driver rules do not match the supplied rule pack".into());
            }
            let now = input["now"]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("now must be a finite number supplied by the host")?;
            let result = d.handle(now, &input["input"], rules)?;
            Ok(json!({"driver":d,"outputs":result}))
        }
        "carry" => {
            let mut d: Driver = decode(input, "driver")?;
            d.carry(&decode(input, "old")?);
            Ok(json!({"driver":d}))
        }
        "settings" => {
            let d: Driver = decode(input, "driver")?;
            Ok(d.settings_block())
        }
        "on_update" => {
            let p: Plan = decode(input, "plan")?;
            let mut slot = input["slot"].clone();
            let changed = &input["changed"];
            let mut write = false;
            let mut fire = V::Null;
            if changed.is_null() {
                write = true
            } else if p.slot_kind.as_deref() == Some("event") {
                if p.depends_on
                    .first()
                    .is_some_and(|c| codes(changed).contains(c))
                {
                    fire = p.read(&input["status"], 0.0, rules)?["event"].clone();
                    write = truth(&fire);
                }
            } else if p.slot_kind.as_deref() == Some("delta") {
                let code = &p.depends_on[0];
                let ts = &input["timestamps"][code];
                let raw = &input["status"][code];
                if codes(changed).contains(code)
                    && !ts.is_null()
                    && *ts != slot["last_ts"]
                    && !raw.is_null()
                {
                    slot["total"] = json!(n(&slot["total"]) + n(raw));
                    slot["last_ts"] = ts.clone();
                    write = true;
                }
            } else {
                write = p.update_all || p.depends_on.iter().any(|c| codes(changed).contains(c));
            }
            Ok(json!({"slot":slot,"write_state":write,"fire":fire}))
        }
        "merge" => {
            let mut value = json!({});
            for mapping in arr(&input["mappings"]) {
                merge(&mut value, &mapping);
            }
            Ok(value)
        }
        "unused" => {
            let a = Adapter {
                entries: decode(input, "entries")?,
                ..Default::default()
            };
            let mut plans = vec![];
            let consumed = codes(&input["consumed"]);
            let a = Adapter {
                entries: a
                    .entries
                    .into_iter()
                    .filter(|(_, e)| !consumed.contains(&e.0))
                    .collect(),
                ..a
            };
            entity::unused(&input["device"], &a, &mut plans, rules)?;
            Ok(json!(plans))
        }
        "quirk_schema" => {
            let mut d = input["device"].clone();
            ensure_tables(&mut d);
            let mut dpmap = d.get("dpmap").cloned().unwrap_or(json!({}));
            apply_quirk(&mut d, &mut dpmap, quirk(input, &input["device"], rules));
            d["dpmap"] = dpmap;
            Ok(d)
        }
        "status_quirk" => {
            let mut st = input["status"].clone();
            map_initial_status(&mut st, &input["quirk"]);
            Ok(st)
        }
        "device_info" => Ok(device_info(
            &input["device"],
            quirk(input, &input["device"], rules),
            rules,
        )),
        "classify" => {
            let plans =
                entity::classify(&input["device"], &input["env"], rules, &input["platforms"])?;
            Ok(json!(plans))
        }
        "read" => {
            let p: Plan = decode(input, "plan")?;
            p.read(&input["status"], n(&input["total"]), rules)
        }
        "write" => {
            let p: Plan = decode(input, "plan")?;
            Ok(json!(p.write(
                s(&input["action"]),
                &input["args"],
                &input["status"],
                rules
            )?))
        }
        "check" => match check(
            &input["descriptor"],
            &input["state"],
            s(&input["prop"]),
            &input["value"],
        ) {
            Ok(v) => Ok(json!({"accept":v})),
            Err((code, reason)) => Ok(json!({"reject":code,"reason":reason})),
        },
        "adapter_create" => Ok(json!(Adapter::new(
            &input["device"],
            &input["dpmap"],
            rules
        ))),
        "adapter_read" => {
            let a: Adapter = decode(input, "adapter")?;
            Ok(a.read(&input["dps"], rules))
        }
        "adapter_write" => {
            let a: Adapter = decode(input, "adapter")?;
            let mut present = vec![];
            let mut missing = vec![];
            for c in arr(&input["commands"]) {
                if a.entries.values().any(|e| e.0 == s(&c["code"])) {
                    present.push(c)
                } else {
                    missing.push(c["code"].clone());
                }
            }
            let dps = if present.is_empty() {
                json!({})
            } else {
                a.write(&present)?
            };
            Ok(json!([dps, missing]))
        }
        "strategy_read" => {
            strategy_read(s(&input["name"]), &input["value"], &input["config"], rules)
        }
        "strategy_write" => strategy_write(s(&input["name"]), &input["value"], &input["config"]),
        _ => Err("unknown engine operation".into()),
    }
}
