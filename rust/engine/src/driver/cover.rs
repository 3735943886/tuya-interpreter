//! Cover settings and the motion tracker that derives a cover's `cover_state`.
use crate::entity::Entity;
use crate::{assembly::*, data::*, plan::Plan};
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};

#[derive(Clone, Serialize, Deserialize)]
pub struct Cover {
    pub plan: usize,
    pub prefix: String,
    pub group: Option<String>,
    pub hazardous: bool,
    pub defaults: V,
    pub settings: V,
    pub cur: Option<Role>,
    pub setp: Option<Role>,
    pub ins: Option<Role>,
    pub tilt: Option<Role>,
    pub position_choice: bool,
    pub switches: Vec<String>,
    /// Map names of the command words this cover's instruction may use, preferred first.
    pub words: V,
}
impl Cover {
    /// Where `cover_state` comes from; `None` gives the cover no `cover_state` at all.
    pub fn source(&self) -> Option<&str> {
        self.settings["state_source"].as_str()
    }
    /// Motion can be inferred only from a position and something that says a move began.
    pub fn can_infer(&self) -> bool {
        self.cur.is_some() && (self.separate() || self.ins.is_some())
    }
    pub fn separate(&self) -> bool {
        self.setp
            .as_ref()
            .is_some_and(|r| self.cur.as_ref().is_none_or(|c| c.code != r.code))
    }
    pub fn applies(&self, k: &str) -> bool {
        match k {
            "position_from_target" => self.position_choice,
            "state_source" => self.cur.is_some() || self.ins.is_some(),
            "invert_reported_motion" => self.ins.is_some() && self.source() == Some("control"),
            "invert_position" => self.cur.is_some(),
            "invert_set_position" => self.separate(),
            "invert_control" => self.ins.is_some() && self.setp.is_none(),
            _ => true,
        }
    }
    pub fn value(&self, k: &str) -> V {
        if k == "state_source" {
            json!(self.source())
        } else {
            self.settings[k].clone()
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Motion {
    pub prop: String,
    pub group: Option<String>,
    pub command: Option<String>,
    pub target_code: Option<String>,
    pub position: Option<String>,
    pub words: V,
    pub settle: f64,
    pub mode: String,
    pub invert_report: bool,
    pub state: Option<String>,
    pub target: Option<f64>,
    pub by_word: bool,
    pub last: Option<f64>,
}
impl Motion {
    pub fn pos(&self, st: &V) -> Option<f64> {
        let v = st.get(self.position.as_deref()?)?;
        if !v.is_number() {
            return None;
        }
        Some(n(v))
    }
    pub fn reset(&mut self) {
        self.state = None;
        self.target = None;
        self.by_word = false;
        self.last = None;
    }
    pub fn update(
        &mut self,
        st: &V,
        changed: &[String],
        active: bool,
        timer: bool,
    ) -> (V, Option<Option<f64>>) {
        let pos = self.pos(st);
        let last = self.last;
        if pos.is_some() {
            self.last = pos;
        }
        let touched = |code: &Option<String>| code.as_ref().is_some_and(|c| changed.contains(c));
        let mut state = None;
        let mut timer_out = None;
        if self.mode != "inferred" {
            if !touched(&self.command) && !touched(&self.position) {
                return (json!({}), None);
            }
            if active && self.mode == "control" && touched(&self.command) {
                let word = &st[self.command.as_deref().unwrap_or("")];
                if *word == self.words["stop"] {
                    state = Some("stopped".to_string());
                } else if *word == self.words["open"] {
                    state = Some(
                        if self.invert_report {
                            "closing"
                        } else {
                            "opening"
                        }
                        .to_string(),
                    );
                } else if *word == self.words["close"] {
                    state = Some(
                        if self.invert_report {
                            "opening"
                        } else {
                            "closing"
                        }
                        .to_string(),
                    );
                }
                if let Some(ref s) = state {
                    self.state = Some(s.clone());
                    return (json!({self.prop.clone():s}), None);
                }
            }
            if active
                && self.mode == "control"
                && self
                    .state
                    .as_deref()
                    .is_some_and(|s| s == "opening" || s == "closing")
            {
                return (json!({}), None);
            }
            self.reset();
            return (
                json!({self.prop.clone():pos.map(|p|if p<=0.0{"closed"}else if p>=100.0{"open"}else{"stopped"})}),
                None,
            );
        }
        if timer {
            if self
                .state
                .as_deref()
                .is_some_and(|s| s == "opening" || s == "closing")
            {
                state = Some("stopped".into());
            } else {
                return (json!({}), None);
            }
        } else if !active {
            if pos.is_some()
                && (touched(&self.command) || touched(&self.position) || touched(&self.target_code))
            {
                state = Some("stopped".into());
            }
        } else {
            if touched(&self.command) {
                let word = &st[self.command.as_deref().unwrap_or("")];
                if *word == self.words["stop"] {
                    state = Some("stopped".into());
                } else if *word == self.words["open"] {
                    state = Some("opening".into());
                    self.target = Some(100.0);
                    self.by_word = true;
                } else if *word == self.words["close"] {
                    state = Some("closing".into());
                    self.target = Some(0.0);
                    self.by_word = true;
                }
            } else if touched(&self.target_code)
                && let Some(position) = pos
            {
                let raw = &st[self.target_code.as_deref().unwrap_or("")];
                if raw.is_number() {
                    let target = n(raw);
                    self.target = Some(target);
                    self.by_word = false;
                    state = Some(
                        if target == position {
                            "stopped"
                        } else if target > position {
                            "opening"
                        } else {
                            "closing"
                        }
                        .into(),
                    );
                }
            } else if touched(&self.position)
                && let Some(p) = pos
                && self
                    .state
                    .as_deref()
                    .is_some_and(|s| s == "opening" || s == "closing")
            {
                if self.by_word
                    && let Some(last) = last
                    && last != p
                {
                    let heading = if p > last { "opening" } else { "closing" };
                    if self.state.as_deref() != Some(heading) {
                        state = Some(heading.into());
                        self.target = Some(if heading == "opening" { 100.0 } else { 0.0 });
                    }
                }
                let moving = state.as_deref().or(self.state.as_deref());
                if self.target.is_some_and(|t| {
                    if moving == Some("opening") {
                        p >= t
                    } else {
                        p <= t
                    }
                }) {
                    state = Some("stopped".into());
                }
            } else if touched(&self.position) && pos.is_some() {
                state = Some("stopped".into());
            }
            if pos.is_some_and(|p| {
                (p <= 0.0 && state.as_deref().or(self.state.as_deref()) == Some("closing"))
                    || (p >= 100.0 && state.as_deref().or(self.state.as_deref()) == Some("opening"))
            }) {
                state = Some("stopped".into());
            }
        }
        let moving = state
            .as_deref()
            .or(self.state.as_deref())
            .is_some_and(|s| s == "opening" || s == "closing")
            && state.as_deref() != Some("stopped");
        if self.settle > 0.0 && !timer {
            timer_out = Some(if moving { Some(self.settle) } else { None });
        }
        let mut vals = json!({});
        if let Some(new) = state {
            self.state = Some(new.clone());
            let out = if new == "stopped" {
                self.target = None;
                self.by_word = false;
                pos.map(|p| {
                    if p <= 0.0 {
                        "closed"
                    } else if p >= 100.0 {
                        "open"
                    } else {
                        "stopped"
                    }
                })
                .unwrap_or("stopped")
                .to_string()
            } else {
                new
            };
            vals[&self.prop] = json!(out);
        }
        (vals, timer_out)
    }
}
pub(super) fn own(block: &V, group: &Option<String>) -> V {
    let c = &block["cover"];
    if let Some(g) = group {
        c[g].clone()
    } else {
        c.clone()
    }
}
pub(super) fn make_covers(
    a: &Assembly,
    plans: &[Plan],
    block: &V,
    settings: bool,
    rules: &V,
) -> Vec<Cover> {
    a.covers
        .iter()
        .map(|(idx, pre, group, haz)| {
            let p = &plans[*idx];
            let spec = Entity::of(rules, &p.platform)
                .map(|e| e.field("cover"))
                .unwrap_or(&V::Null);
            let role = |k: &str| p.role(s(&spec[k])).cloned();
            let mut defaults = rules["cover_defaults"].clone();
            if settings {
                defaults["state_source"] = json!("inferred");
            }
            let mut cfg = defaults.clone();
            let own = own(block, group);
            for (k, v) in obj(&own) {
                if defaults.get(&k).is_some() {
                    cfg[&k] = v;
                }
            }
            let cur = codes(&spec["position"])
                .iter()
                .find_map(|k| p.role(k))
                .cloned();
            let setp = role("target");
            let choice = cur.is_some()
                && setp
                    .as_ref()
                    .is_some_and(|r| cur.as_ref().is_some_and(|c| c.code != r.code));
            Cover {
                plan: *idx,
                prefix: pre.clone(),
                group: group.clone(),
                hazardous: *haz,
                defaults,
                settings: cfg,
                cur,
                setp,
                ins: role("instruction"),
                tilt: role("tilt"),
                position_choice: choice,
                switches: vec![],
                words: spec["words"].clone(),
            }
        })
        .collect()
}
