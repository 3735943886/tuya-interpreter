use crate::error::OrEval;
use crate::protocol::SlotKind;
use crate::runtime::{fingerprint, validated};
use crate::{Driver as LiveDriver, DriverOptions, EngineError, Input, RulePack};
use crate::{adapter::*, data::*, driver::Driver, driver::check, entity, plan::Plan};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value as V, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
static PACKS: OnceLock<Mutex<HashMap<String, Arc<RulePack>>>> = OnceLock::new();
fn decode<T: DeserializeOwned>(input: &V, field: &str) -> Result<T, EngineError> {
    T::deserialize(&input[field]).map_err(|e| EngineError::InvalidInput(format!("{field}: {e}")))
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
    RemoveRules,
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
    Open,
}
/// Operations on a driver kept in engine memory by `open`.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Live {
    Drive,
    View,
    Snapshot,
    CarryHandle,
    SameConfig,
    Close,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Op {
    Plain(Plain),
    Ruled(Ruled),
    Live(Live),
}
/// The rule pack a request runs with.
enum Rules<'a> {
    Registered(Arc<RulePack>),
    Inline { rules: &'a V, fingerprint: String },
}
impl<'a> Rules<'a> {
    fn resolve(input: &'a V) -> Result<Self, EngineError> {
        let rules = &input["rules"];
        if rules.is_null() {
            return registry()?
                .get(s(&input["rules_id"]))
                .cloned()
                .map(Rules::Registered)
                .ok_or_else(|| {
                    EngineError::InvalidRules(
                        "rules are required or rules_id is not registered".into(),
                    )
                });
        }
        Ok(Rules::Inline {
            rules,
            fingerprint: validated(rules)?,
        })
    }
    fn rules(&self) -> &V {
        match self {
            Rules::Registered(pack) => pack.rules(),
            Rules::Inline { rules, .. } => rules,
        }
    }
    /// The pack itself, for a driver that outlives this request.
    fn pack(&self) -> Result<Arc<RulePack>, EngineError> {
        match self {
            Rules::Registered(pack) => Ok(pack.clone()),
            Rules::Inline { rules, .. } => Ok(Arc::new(RulePack::new((*rules).clone())?)),
        }
    }
    fn fingerprint(&self) -> &str {
        match self {
            Rules::Registered(pack) => pack.fingerprint(),
            Rules::Inline { fingerprint, .. } => fingerprint,
        }
    }
}
fn registry() -> Result<MutexGuard<'static, HashMap<String, Arc<RulePack>>>, EngineError> {
    PACKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| EngineError::InvalidRules("rule registry poisoned".into()))
}
fn install(input: &V) -> Result<V, EngineError> {
    let rules = &input["rules"];
    let id = input["rules_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| EngineError::InvalidInput("rules_id must be a nonempty string".into()))?;
    let conflict =
        || EngineError::InvalidRules("rule id already registered with different content".into());
    if let Some(old) = registry()?.get(id).cloned() {
        // The fingerprint is the ordered content, so equal fingerprints are the same pack.
        return if old.fingerprint() == fingerprint(rules) {
            Ok(json!(true))
        } else {
            Err(conflict())
        };
    }
    // Validation and hashing never hold the global registry lock.
    let pack = Arc::new(RulePack::new(rules.clone())?);
    let mut packs = registry()?;
    match packs.get(id) {
        Some(old) if old.fingerprint() != pack.fingerprint() => return Err(conflict()),
        Some(_) => {}
        None => {
            packs.insert(id.into(), pack);
        }
    }
    Ok(json!(true))
}
pub fn request(input: &V) -> Result<V, EngineError> {
    let op: Op = serde_json::from_value(input["op"].clone())
        .map_err(|_| "unknown engine operation")
        .or_eval()?;
    match op {
        Op::Plain(op) => plain(op, input),
        Op::Ruled(op) => ruled(op, input, &Rules::resolve(input)?),
        Op::Live(op) => live(op, input),
    }
}

/// Drivers kept in engine memory, so each message carries only its input and outputs.
#[derive(Default)]
struct Drivers {
    next: u64,
    open: HashMap<u64, Arc<Mutex<LiveDriver>>>,
}
static DRIVERS: OnceLock<Mutex<Drivers>> = OnceLock::new();
fn drivers() -> Result<MutexGuard<'static, Drivers>, EngineError> {
    DRIVERS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| EngineError::InvalidInput("driver registry poisoned".into()))
}
fn open_driver(input: &V, pack: &Rules) -> Result<V, EngineError> {
    let options = if input["options"].is_null() {
        DriverOptions::default()
    } else {
        decode(input, "options")?
    };
    let driver = LiveDriver::new(input["device"].clone(), pack.pack()?, options)?;
    let mut registry = drivers()?;
    registry.next += 1;
    let handle = registry.next;
    registry.open.insert(handle, Arc::new(Mutex::new(driver)));
    Ok(json!({"handle": handle}))
}
/// The driver a request's `field` names. The registry lock is released before the driver is used.
fn opened(input: &V, field: &str) -> Result<(u64, Arc<Mutex<LiveDriver>>), EngineError> {
    let handle = input[field]
        .as_u64()
        .ok_or_else(|| EngineError::InvalidInput(format!("{field} must be a driver handle")))?;
    let driver =
        drivers()?.open.get(&handle).cloned().ok_or_else(|| {
            EngineError::InvalidInput(format!("{field}: no open driver {handle}"))
        })?;
    Ok((handle, driver))
}
fn lock(driver: &Mutex<LiveDriver>) -> Result<MutexGuard<'_, LiveDriver>, EngineError> {
    driver
        .lock()
        .map_err(|_| EngineError::InvalidInput("driver poisoned".into()))
}
fn status(d: &LiveDriver) -> V {
    json!({"linked": d.linked(), "synced": d.synced(), "timers": d.timers()})
}
fn live(op: Live, input: &V) -> Result<V, EngineError> {
    if let Live::Close = op {
        let (handle, _) = opened(input, "handle")?;
        return Ok(json!(drivers()?.open.remove(&handle).is_some()));
    }
    let (handle, driver) = opened(input, "handle")?;
    if let Live::CarryHandle = op {
        let (old_handle, old) = opened(input, "old")?;
        if old_handle != handle {
            // A copy of the old driver, so two driver locks are never held at once.
            let old = lock(&old)?.clone();
            lock(&driver)?.carry(&old);
        }
        return Ok(json!(true));
    }
    if let Live::SameConfig = op {
        let (other_handle, other) = opened(input, "other")?;
        if other_handle == handle {
            return Ok(json!(true));
        }
        let other = lock(&other)?.clone();
        return Ok(json!(lock(&driver)?.same_config(&other)));
    }
    let mut d = lock(&driver)?;
    Ok(match op {
        Live::Drive => {
            let now = input["now"].as_f64().unwrap_or(f64::NAN);
            let outputs = d.handle(now, decode(input, "input")?)?;
            let mut result = status(&d);
            result["outputs"] = json!(outputs);
            result
        }
        Live::View => {
            let mut result = status(&d);
            result["descriptor"] = d.descriptor().clone();
            result["dps"] = d.dps().clone();
            result
        }
        Live::Snapshot => d.snapshot(),
        Live::CarryHandle | Live::SameConfig | Live::Close => unreachable!("handled above"),
    })
}
fn plain(op: Plain, input: &V) -> Result<V, EngineError> {
    match op {
        Plain::MergeRules => crate::rules::compose(input).map_err(EngineError::InvalidRules),
        Plain::InstallRules => install(input),
        Plain::RemoveRules => {
            let id = input["rules_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    EngineError::InvalidInput("rules_id must be a nonempty string".into())
                })?;
            Ok(json!(registry()?.remove(id).is_some()))
        }
        Plain::Carry => {
            let mut d: Driver = decode(input, "driver")?;
            d.validate_state()?;
            d.carry(&decode(input, "old")?);
            Ok(json!({"driver":d}))
        }
        Plain::Settings => {
            let d: Driver = decode(input, "driver")?;
            d.validate_state()?;
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
                if a.entries.values().any(|e| e.code == s(&c["code"])) {
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
fn ruled(op: Ruled, input: &V, pack: &Rules) -> Result<V, EngineError> {
    let rules = pack.rules();
    match op {
        Ruled::Create => {
            let options: DriverOptions = if input["options"].is_null() {
                DriverOptions::default()
            } else {
                decode(input, "options")?
            };
            let mut d = Driver::new(&input["device"], &options, rules)?;
            d.rules_fingerprint = pack.fingerprint().into();
            Ok(json!({"driver":d}))
        }
        Ruled::Handle => {
            let mut d: Driver = decode(input, "driver")?;
            d.validate_state()?;
            if d.rules_fingerprint != pack.fingerprint() {
                return Err(EngineError::InvalidRules(
                    "driver rules do not match the supplied rule pack".into(),
                ));
            }
            let now = input["now"]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    EngineError::InvalidInput(
                        "now must be a finite number supplied by the host".into(),
                    )
                })?;
            let event: Input = decode(input, "input")?;
            let result = d.handle(now, &event, rules)?;
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
            } else if p.slot_kind == Some(SlotKind::Event) {
                if let Some(event) = p
                    .event(&input["status"], &codes(changed), rules)
                    .or_eval()?
                {
                    fire = event;
                    write = true;
                }
            } else if p.slot_kind == Some(SlotKind::Delta) {
                if let Some(code) = p.depends_on.first() {
                    let mut total = n(&slot["total"]);
                    let mut last_ts = slot["last_ts"].clone();
                    write = crate::plan::accumulate(
                        &mut total,
                        &mut last_ts,
                        &input["status"][code],
                        &input["timestamps"][code],
                        codes(changed).contains(code),
                    );
                    if write {
                        slot["total"] = json!(total);
                        slot["last_ts"] = last_ts;
                    }
                }
            } else {
                write = p.affected(&codes(changed));
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
                    .filter(|(_, e)| !consumed.contains(&e.code))
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
                .map_err(EngineError::Evaluation)
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
        Ruled::Open => open_driver(input, pack),
        Ruled::StrategyRead => {
            strategy_read(s(&input["name"]), &input["value"], &input["config"], rules)
                .map_err(EngineError::Evaluation)
        }
    }
}
