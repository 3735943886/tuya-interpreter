use crate::{adapter::*, data::*, driver::Driver, driver::check, entity, plan::Plan};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value as V, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
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
/// Operations that need no rule pack.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Plain {
    MergeRules,
    InstallRules,
    Carry,
    Settings,
    Merge,
    StatusQuirk,
    Check,
    AdapterWrite,
    StrategyWrite,
}
/// Operations that interpret a device with a rule pack (inline `rules` or a registered `rules_id`).
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Ruled {
    Create,
    Handle,
    OnUpdate,
    Unused,
    QuirkSchema,
    DeviceInfo,
    Classify,
    Read,
    Write,
    AdapterCreate,
    AdapterRead,
    StrategyRead,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Op {
    Plain(Plain),
    Ruled(Ruled),
}
/// The rule pack a request runs with.
enum Rules<'a> {
    Registered(Arc<Pack>),
    Inline { rules: &'a V, fingerprint: String },
}
impl<'a> Rules<'a> {
    fn resolve(input: &'a V) -> Result<Self, String> {
        let rules = &input["rules"];
        if rules.is_null() {
            return registry()?
                .get(s(&input["rules_id"]))
                .cloned()
                .map(Rules::Registered)
                .ok_or_else(|| "rules are required or rules_id is not registered".into());
        }
        if rules["version"] != 1 {
            return Err("unsupported rule version".into());
        }
        crate::rules::validate(rules)?;
        Ok(Rules::Inline {
            rules,
            fingerprint: fingerprint(rules),
        })
    }
    fn rules(&self) -> &V {
        match self {
            Rules::Registered(pack) => &pack.rules,
            Rules::Inline { rules, .. } => rules,
        }
    }
    fn fingerprint(&self) -> &str {
        match self {
            Rules::Registered(pack) => &pack.fingerprint,
            Rules::Inline { fingerprint, .. } => fingerprint,
        }
    }
}
fn registry() -> Result<MutexGuard<'static, HashMap<String, Arc<Pack>>>, String> {
    PACKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "rule registry poisoned".into())
}
fn install(input: &V) -> Result<V, String> {
    let rules = input["rules"].clone();
    if rules["version"] != 1 {
        return Err("unsupported rule version".into());
    }
    let id = input["rules_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("rules_id must be a nonempty string")?
        .to_string();
    let mut packs = registry()?;
    if let Some(old) = packs.get(&id) {
        if !ordered_eq(&old.rules, &rules) {
            return Err("rule id already registered with different content".into());
        }
        return Ok(json!(true));
    }
    crate::rules::validate(&rules)?;
    let fingerprint = fingerprint(&rules);
    packs.insert(id, Arc::new(Pack { rules, fingerprint }));
    Ok(json!(true))
}
pub fn request(input: &V) -> Result<V, String> {
    let op: Op =
        serde_json::from_value(input["op"].clone()).map_err(|_| "unknown engine operation")?;
    match op {
        Op::Plain(op) => plain(op, input),
        Op::Ruled(op) => ruled(op, input, &Rules::resolve(input)?),
    }
}
fn plain(op: Plain, input: &V) -> Result<V, String> {
    match op {
        Plain::MergeRules => crate::rules::compose(input),
        Plain::InstallRules => install(input),
        Plain::Carry => {
            let mut d: Driver = decode(input, "driver")?;
            d.carry(&decode(input, "old")?);
            Ok(json!({"driver":d}))
        }
        Plain::Settings => {
            let d: Driver = decode(input, "driver")?;
            Ok(d.settings_block())
        }
        Plain::Merge => {
            let mut value = json!({});
            for mapping in arr(&input["mappings"]) {
                merge(&mut value, &mapping);
            }
            Ok(value)
        }
        Plain::StatusQuirk => {
            let mut st = input["status"].clone();
            map_initial_status(&mut st, &input["quirk"]);
            Ok(st)
        }
        Plain::Check => match check(
            &input["descriptor"],
            &input["state"],
            s(&input["prop"]),
            &input["value"],
        ) {
            Ok(v) => Ok(json!({"accept":v})),
            Err((code, reason)) => Ok(json!({"reject":code,"reason":reason})),
        },
        Plain::AdapterWrite => {
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
        Plain::StrategyWrite => {
            strategy_write(s(&input["name"]), &input["value"], &input["config"])
        }
    }
}
fn ruled(op: Ruled, input: &V, pack: &Rules) -> Result<V, String> {
    let rules = pack.rules();
    match op {
        Ruled::Create => {
            let mut d = Driver::new(&input["device"], &input["options"], rules)?;
            if let Rules::Registered(_) = pack {
                d.rules_id = s(&input["rules_id"]).into();
            }
            d.rules_fingerprint = pack.fingerprint().into();
            Ok(json!({"driver":d}))
        }
        Ruled::Handle => {
            let mut d: Driver = decode(input, "driver")?;
            if d.rules_fingerprint != pack.fingerprint() {
                return Err("driver rules do not match the supplied rule pack".into());
            }
            let now = input["now"]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("now must be a finite number supplied by the host")?;
            let result = d.handle(now, &input["input"], rules)?;
            Ok(json!({"driver":d,"outputs":result}))
        }
        Ruled::OnUpdate => {
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
        Ruled::Unused => {
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
        Ruled::QuirkSchema => {
            let mut d = input["device"].clone();
            ensure_tables(&mut d);
            let mut dpmap = d.get("dpmap").cloned().unwrap_or(json!({}));
            apply_quirk(&mut d, &mut dpmap, quirk(input, &input["device"], rules));
            d["dpmap"] = dpmap;
            Ok(d)
        }
        Ruled::DeviceInfo => Ok(device_info(
            &input["device"],
            quirk(input, &input["device"], rules),
            rules,
        )),
        Ruled::Classify => {
            let plans =
                entity::classify(&input["device"], &input["env"], rules, &input["platforms"])?;
            Ok(json!(plans))
        }
        Ruled::Read => {
            let p: Plan = decode(input, "plan")?;
            p.read(&input["status"], n(&input["total"]), rules)
        }
        Ruled::Write => {
            let p: Plan = decode(input, "plan")?;
            Ok(json!(p.write(
                s(&input["action"]),
                &input["args"],
                &input["status"],
                rules
            )?))
        }
        Ruled::AdapterCreate => Ok(json!(Adapter::new(
            &input["device"],
            &input["dpmap"],
            rules
        ))),
        Ruled::AdapterRead => {
            let a: Adapter = decode(input, "adapter")?;
            Ok(a.read(&input["dps"], rules))
        }
        Ruled::StrategyRead => {
            strategy_read(s(&input["name"]), &input["value"], &input["config"], rules)
        }
    }
}
