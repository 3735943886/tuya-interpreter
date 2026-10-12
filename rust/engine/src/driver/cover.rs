//! Cover settings and the motion tracker that derives a cover's `cover_state`.
use crate::entity::Entity;
use crate::{MotionState, StateSource};
use crate::{assembly::*, data::*, plan::Plan};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value as V, json};

/// A cover's settings. Their meaning is fixed here; the rule pack's `cover_defaults` supplies the
/// defaults and `cover_switches` the ones a device can expose as properties. Fields keep the rule
/// pack's declaration order, which settings blocks follow.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoverSettings {
    pub invert_position: bool,
    pub invert_set_position: bool,
    pub invert_control: bool,
    /// Seconds after the last movement at which an inferred motion counts as stopped; 0 for none.
    #[serde(serialize_with = "canonical")]
    pub settle: f64,
    pub invert_tilt: bool,
    pub position_from_target: bool,
    /// Where `cover_state` comes from; `None` gives the cover no `cover_state` at all.
    pub state_source: Option<StateSource>,
    pub invert_reported_motion: bool,
}
fn canonical<S: Serializer>(x: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    number(*x).serialize(serializer)
}
impl CoverSettings {
    /// The rule pack's `cover_defaults`.
    pub fn from_rules(defaults: &V) -> Result<Self, String> {
        Self::deserialize(defaults).map_err(|e| format!("cover_defaults: {e}"))
    }
    /// Whether `key` names a setting.
    pub fn known(key: &str) -> bool {
        json!(Self::default()).get(key).is_some()
    }
    pub fn get(&self, key: &str) -> V {
        json!(self)[key].clone()
    }
    /// Set one setting from JSON; an unknown key or a value of the wrong type changes nothing.
    pub fn set(&mut self, key: &str, value: &V) -> bool {
        let mut all = json!(self);
        if all.get(key).is_none() {
            return false;
        }
        all[key] = value.clone();
        match Self::deserialize(&all) {
            Ok(settings) => {
                *self = settings;
                true
            }
            Err(_) => false,
        }
    }
    /// Apply the settings an override `cover` block names; other members are ignored.
    pub fn apply(&mut self, block: &V) {
        for (key, value) in fields(block) {
            self.set(key, value);
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Cover {
    pub plan: usize,
    pub prefix: String,
    pub group: Option<String>,
    pub hazardous: bool,
    pub defaults: CoverSettings,
    pub settings: CoverSettings,
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
    pub fn source(&self) -> Option<StateSource> {
        self.settings.state_source
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
            "invert_reported_motion" => {
                self.ins.is_some() && self.source() == Some(StateSource::Control)
            }
            "invert_position" => self.cur.is_some(),
            "invert_set_position" => self.separate(),
            "invert_control" => self.ins.is_some() && self.setp.is_none(),
            _ => true,
        }
    }
    pub fn value(&self, k: &str) -> V {
        self.settings.get(k)
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
    pub mode: StateSource,
    pub invert_report: bool,
    pub state: Option<MotionState>,
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
        if self.mode != StateSource::Inferred {
            if !touched(&self.command) && !touched(&self.position) {
                return (json!({}), None);
            }
            if active && self.mode == StateSource::Control && touched(&self.command) {
                let word = &st[self.command.as_deref().unwrap_or("")];
                if *word == self.words["stop"] {
                    state = Some(MotionState::Stopped);
                } else if *word == self.words["open"] {
                    state = Some(if self.invert_report {
                        MotionState::Closing
                    } else {
                        MotionState::Opening
                    });
                } else if *word == self.words["close"] {
                    state = Some(if self.invert_report {
                        MotionState::Opening
                    } else {
                        MotionState::Closing
                    });
                }
                if let Some(ref s) = state {
                    self.state = Some(*s);
                    return (json!({self.prop.clone():s}), None);
                }
            }
            if active
                && self.mode == StateSource::Control
                && self.state.is_some_and(MotionState::moving)
            {
                return (json!({}), None);
            }
            self.reset();
            return (
                json!({self.prop.clone():pos.map(MotionState::resting)}),
                None,
            );
        }
        if timer {
            if self.state.is_some_and(MotionState::moving) {
                state = Some(MotionState::Stopped);
            } else {
                return (json!({}), None);
            }
        } else if !active {
            if pos.is_some()
                && (touched(&self.command) || touched(&self.position) || touched(&self.target_code))
            {
                state = Some(MotionState::Stopped);
            }
        } else {
            if touched(&self.command) {
                let word = &st[self.command.as_deref().unwrap_or("")];
                if *word == self.words["stop"] {
                    state = Some(MotionState::Stopped);
                } else if *word == self.words["open"] {
                    state = Some(MotionState::Opening);
                    self.target = Some(100.0);
                    self.by_word = true;
                } else if *word == self.words["close"] {
                    state = Some(MotionState::Closing);
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
                    state = Some(if target == position {
                        MotionState::Stopped
                    } else if target > position {
                        MotionState::Opening
                    } else {
                        MotionState::Closing
                    });
                }
            } else if touched(&self.position)
                && let Some(p) = pos
                && self.state.is_some_and(MotionState::moving)
            {
                if self.by_word
                    && let Some(last) = last
                    && last != p
                {
                    let heading = if p > last {
                        MotionState::Opening
                    } else {
                        MotionState::Closing
                    };
                    if self.state != Some(heading) {
                        state = Some(heading);
                        self.target = Some(if heading == MotionState::Opening {
                            100.0
                        } else {
                            0.0
                        });
                    }
                }
                let moving = state.or(self.state);
                if self.target.is_some_and(|t| {
                    if moving == Some(MotionState::Opening) {
                        p >= t
                    } else {
                        p <= t
                    }
                }) {
                    state = Some(MotionState::Stopped);
                }
            } else if touched(&self.position) && pos.is_some() {
                state = Some(MotionState::Stopped);
            }
            if pos.is_some_and(|p| {
                (p <= 0.0 && state.or(self.state) == Some(MotionState::Closing))
                    || (p >= 100.0 && state.or(self.state) == Some(MotionState::Opening))
            }) {
                state = Some(MotionState::Stopped);
            }
        }
        let moving = state.or(self.state).is_some_and(MotionState::moving);
        if self.settle > 0.0 && !timer {
            timer_out = Some(if moving { Some(self.settle) } else { None });
        }
        let mut vals = json!({});
        if let Some(new) = state {
            self.state = Some(new);
            let out = if new == MotionState::Stopped {
                self.target = None;
                self.by_word = false;
                pos.map(MotionState::resting)
                    .unwrap_or(MotionState::Stopped)
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
            // Validated with the rule pack.
            let mut defaults =
                CoverSettings::from_rules(&rules["cover_defaults"]).unwrap_or_default();
            if settings {
                defaults.state_source = Some(StateSource::Inferred);
            }
            let mut cfg = defaults.clone();
            cfg.apply(&own(block, group));
            let cur = codes(&spec["position"])
                .iter()
                .find_map(|k| p.role(k))
                .cloned();
            let mut cover = Cover {
                plan: *idx,
                prefix: pre.clone(),
                group: group.clone(),
                hazardous: *haz,
                defaults,
                settings: cfg,
                cur,
                setp: role("target"),
                ins: role("instruction"),
                tilt: role("tilt"),
                position_choice: false,
                switches: vec![],
                words: spec["words"].clone(),
            };
            cover.position_choice = cover.cur.is_some() && cover.separate();
            cover
        })
        .collect()
}
