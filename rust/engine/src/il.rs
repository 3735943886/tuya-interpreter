//! The il-mqtt wire (il-mqtt.md): topic layout, payload encoding and the host actions a driver's
//! outputs become. Pure mapping; the host owns the connection.
use crate::data::*;
use serde::Deserialize;
use serde_json::{Value as V, json};

const QOS: u8 = 1;

/// The default topic layout (M-7) under a prefix, for one producer `source`.
#[derive(Deserialize)]
#[serde(default)]
pub struct Topics {
    prefix: String,
    source: String,
}

impl Default for Topics {
    fn default() -> Self {
        Self {
            prefix: "il".into(),
            source: "tuya".into(),
        }
    }
}

impl Topics {
    /// The request's `topics`, or the default layout.
    pub fn of(input: &V) -> Result<Self, String> {
        serde_json::from_value(input.get("topics").cloned().unwrap_or(json!({})))
            .map_err(|e| format!("topics: {e}"))
    }

    pub fn descriptor(&self, id: &str) -> String {
        format!("{}/{id}", self.prefix)
    }

    pub fn state(&self, id: &str, prop: &str) -> String {
        format!("{}/{id}/{prop}", self.prefix)
    }

    pub fn reject(&self, id: &str) -> String {
        format!("{}/{id}/reject", self.prefix)
    }

    pub fn presence(&self) -> String {
        format!("{}/_producer/{}", self.prefix, self.source)
    }

    pub fn set_subscription(&self) -> String {
        format!("{}/+/+/set", self.prefix)
    }

    /// `(device id, property)` of a write topic.
    pub fn parse_set(&self, topic: &str) -> Option<(String, String)> {
        let rest = topic.strip_prefix(&self.prefix)?.strip_prefix('/')?;
        match rest.split('/').collect::<Vec<_>>()[..] {
            [id, prop, "set"] if !id.starts_with('_') => Some((id.into(), prop.into())),
            _ => None,
        }
    }

    fn publish(&self, topic: String, payload: String, retain: bool) -> V {
        json!({"kind":"publish","topic":topic,"payload":payload,"retain":retain,"qos":QOS})
    }

    /// Retained clears of a device's property values and, with `descriptor`, of the device itself.
    pub fn clear<'p>(
        &self,
        id: &str,
        props: impl IntoIterator<Item = &'p str>,
        descriptor: bool,
    ) -> Vec<V> {
        let mut out: Vec<V> = props
            .into_iter()
            .map(|p| self.publish(self.state(id, p), String::new(), true))
            .collect();
        if descriptor {
            out.push(self.publish(self.descriptor(id), String::new(), true));
        }
        out
    }

    /// The host actions for a device's driver outputs, in order.
    pub fn actions(&self, id: &str, outputs: &[V]) -> Vec<V> {
        outputs
            .iter()
            .filter_map(|o| {
                let prop = s(&o["prop"]);
                Some(match s(&o["type"]) {
                    "descriptor" => {
                        let mut desc = o["desc"].clone();
                        desc["source"] = json!(self.source);
                        self.publish(self.descriptor(id), desc.to_string(), true)
                    }
                    "value" => self.publish(self.state(id, prop), encode(&o["value"]), true),
                    "absent" => self.publish(self.state(id, prop), String::new(), true),
                    "event" => self.publish(self.state(id, prop), encode(&o["kind"]), false),
                    "reject" => self.publish(
                        self.reject(id),
                        json!({"prop":o["prop"],"code":o["code"],"reason":o["reason"]}).to_string(),
                        false,
                    ),
                    "set_timer" => json!({"kind":"schedule","name":o["name"],"after":o["after"]}),
                    "cancel_timer" => json!({"kind":"unschedule","name":o["name"]}),
                    "send_message" if o["channel"] == "set" => {
                        json!({"kind":"bridge","action":"set","dps":o["json"]["dps"]})
                    }
                    "settings_changed" => json!({"kind":"save_settings","block":o["block"]}),
                    _ => return None,
                })
            })
            .collect()
    }
}

/// A state payload: no JSON quoting of strings; an integral number has no fraction (V-5).
pub fn encode(v: &V) -> String {
    match v {
        V::Bool(b) => b.to_string(),
        V::Number(_) if v.is_f64() => {
            let x = n(v);
            if x.fract() == 0.0 && x.abs() < 1e15 {
                (x as i64).to_string()
            } else {
                v.to_string()
            }
        }
        V::String(t) => t.clone(),
        V::Null => String::new(),
        other => other.to_string(),
    }
}

/// A write payload as a command value; the driver's check refuses what does not fit the property.
pub fn decode(ty: &str, payload: &str) -> V {
    match ty {
        "trigger" => V::Null,
        "number" => serde_json::from_str(payload).unwrap_or_else(|_| json!(payload)),
        _ => json!(payload),
    }
}

/// M-2: a device id must be one topic level that is not reserved.
pub fn usable_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('_') && !id.contains(['/', '+', '#', '\0'])
}

/// `{"op":"il","action":...}`: the wire operations a host binding needs.
pub fn request(input: &V) -> Result<V, String> {
    let topics = Topics::of(input)?;
    let id = s(&input["device"]);
    Ok(match s(&input["action"]) {
        "actions" => json!(topics.actions(id, items(&input["outputs"]))),
        "clear" => json!(topics.clear(
            id,
            items(&input["props"]).iter().map(s),
            input["descriptor"] == true
        )),
        "presence" => topics.publish(
            topics.presence(),
            if input["online"] == true {
                "online"
            } else {
                "offline"
            }
            .into(),
            true,
        ),
        "topics" => json!({
            "descriptor": topics.descriptor(id),
            "reject": topics.reject(id),
            "presence": topics.presence(),
            "set_subscription": topics.set_subscription(),
            "state": input.get("prop").map(|p| topics.state(id, s(p))),
        }),
        "parse_set" => json!(topics.parse_set(s(&input["topic"]))),
        "encode" => json!(encode(&input["value"])),
        "decode" => decode(s(&input["type"]), s(&input["payload"])),
        "usable_id" => json!(usable_id(id)),
        other => return Err(format!("unknown il action {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_numbers_have_no_fraction_and_strings_no_quotes() {
        assert_eq!(encode(&json!(50.0)), "50");
        assert_eq!(encode(&json!(21.5)), "21.5");
        assert_eq!(encode(&json!("say \"hi\"")), "say \"hi\"");
        assert_eq!(encode(&json!(true)), "true");
    }

    #[test]
    fn write_topics_name_a_device_and_property() {
        let t = Topics::default();
        assert_eq!(
            t.parse_set("il/dev1/power/set"),
            Some(("dev1".into(), "power".into()))
        );
        assert_eq!(t.parse_set("il/_producer/x/set"), None);
        assert_eq!(t.parse_set("ilx/dev1/power/set"), None);
        assert!(!usable_id("a/b") && usable_id("a.b-c"));
    }
}
