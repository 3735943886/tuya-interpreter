//! Many devices behind one il-mqtt producer: what adopting, removing and feeding a device's
//! driver publishes. The host keeps each device's driver JSON and passes the one concerned.
use crate::data::*;
use crate::driver::Driver;
use crate::il::{self, Topics};
use serde::de::DeserializeOwned;
use serde_json::{Value as V, json};

fn decode<T: DeserializeOwned>(v: &V, field: &str) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|e| format!("{field}: {e}"))
}

fn get() -> V {
    json!({"kind":"bridge","action":"get","dps":null})
}

fn id(d: &Driver) -> &str {
    s(&d.device["id"])
}

fn is_message(input: &V, channel: &str) -> bool {
    input["type"] == "message" && input["channel"] == channel
}

/// Drive a device with `new` from now on. `old`'s timers are cancelled, the properties it had that
/// `new` lacks (and its motion states) are cleared, and `new` is described with `seed` applied.
fn replace(
    old: Option<&Driver>,
    new: &mut Driver,
    seed: Vec<V>,
    now: f64,
    topics: &Topics,
    rules: &V,
) -> Result<Vec<V>, String> {
    let device = id(new).to_string();
    let mut actions = vec![];
    if let Some(old) = old {
        new.carry(old);
        let mut timers = old.timers.clone();
        timers.sort();
        actions.extend(
            timers
                .iter()
                .map(|name| json!({"kind":"unschedule","name":name})),
        );
        let motion = &rules["motion_property"]["role"];
        let props = &new.assembly.descriptor["props"];
        let mut gone: Vec<&str> = fields(&old.assembly.descriptor["props"])
            .filter(|(p, def)| props.get(*p).is_none() || def["role"] == *motion)
            .map(|(p, _)| p.as_str())
            .collect();
        gone.sort_unstable();
        actions.extend(topics.clear(&device, gone, false));
    }
    let outputs = new.handle(now, &json!({"type":"describe","seed":seed}), rules)?;
    actions.extend(topics.actions(&device, &outputs));
    Ok(actions)
}

/// `{"op":"hub","action":...}`.
pub fn request(input: &V, rules: &V) -> Result<V, String> {
    let topics = Topics::of(input)?;
    let now = n(&input["now"]);
    let old: Option<Driver> = match &input["old"] {
        V::Null => None,
        v => Some(decode(v, "old")?),
    };
    Ok(match s(&input["action"]) {
        // A device added, or its record replaced (`reload: false`), or the rules changed under it
        // (`reload: true`, where an unchanged driver is kept and the old values are replayed).
        "adopt" => {
            let mut new: Driver = decode(&input["new"], "new")?;
            if !il::usable_id(id(&new)) {
                return Err(format!(
                    "device id {:?} is not usable as an il-mqtt topic level (M-2)",
                    id(&new)
                ));
            }
            let reload = input["reload"] == true;
            if let Some(old) = old.as_ref().filter(|o| reload && o.same_as(&new)) {
                return Ok(json!({"driver":old,"actions":[],"kept":true}));
            }
            let (linked, synced) = old
                .as_ref()
                .map_or((false, false), |o| (o.linked, o.synced));
            let seed = if reload {
                let mut seed = vec![];
                if linked {
                    seed.push(json!({"type":"connected"}));
                }
                if let Some(o) = old.as_ref().filter(|o| o.synced) {
                    seed.push(json!({"type":"message","channel":"state","json":o.dps}));
                }
                seed
            } else {
                match arr(&input["seed"]) {
                    seed if seed.is_empty() && linked => vec![json!({"type":"connected"})],
                    seed => seed,
                }
            };
            let ask = if reload {
                linked && !synced
            } else {
                seed.iter().any(|i| i["type"] == "connected")
                    && !seed.iter().any(|i| is_message(i, "state"))
            };
            let mut actions = replace(old.as_ref(), &mut new, seed, now, &topics, rules)?;
            if ask {
                actions.push(get());
            }
            json!({"driver":new,"actions":actions,"kept":false})
        }
        // M-11: clear every retained value, then the descriptor.
        "remove" => {
            let old = old.ok_or("remove needs the old driver")?;
            let mut timers = old.timers.clone();
            timers.sort();
            let mut actions: Vec<V> = timers
                .iter()
                .map(|name| json!({"kind":"unschedule","name":name}))
                .collect();
            let props = fields(&old.assembly.descriptor["props"]).map(|(p, _)| p.as_str());
            actions.extend(topics.clear(id(&old), props, true));
            json!({"actions":actions})
        }
        // Bridge input or a timer. A replayed delta is not a new event (M-9); a live connect asks for
        // the state, a retained one only while none has arrived.
        "input" => {
            let mut d: Driver = decode(&input["driver"], "driver")?;
            let inp = &input["input"];
            let retained = input["retained"] == true;
            if retained && inp["type"] == "message" && !is_message(inp, "state") {
                return Ok(json!({"driver":d,"actions":[]}));
            }
            let outputs = d.handle(now, inp, rules)?;
            let mut actions = topics.actions(id(&d), &outputs);
            if inp["type"] == "connected" && !(retained && d.synced) {
                actions.push(get());
            }
            json!({"driver":d,"actions":actions})
        }
        // An il write: decoded by the property's type, then checked and sent by the driver.
        "write" => {
            let mut d: Driver = decode(&input["driver"], "driver")?;
            let prop = s(&input["prop"]);
            let ty = s(&d.assembly.descriptor["props"][prop]["type"]).to_string();
            let value = il::decode(&ty, s(&input["payload"]));
            let outputs = d.handle(
                now,
                &json!({"type":"command","prop":prop,"value":value}),
                rules,
            )?;
            let actions = topics.actions(id(&d), &outputs);
            json!({"driver":d,"actions":actions})
        }
        other => return Err(format!("unknown hub action {other}")),
    })
}
