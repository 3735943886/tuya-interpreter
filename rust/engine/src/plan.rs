use crate::EngineError;
use crate::data::*;
use crate::entity;
use crate::protocol::SlotKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};

/// A classified entity: its resolved roles plus what its spec derived at build time.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Plan {
    pub platform: String,
    pub key: String,
    pub identity: V,
    pub roles: Roles,
    pub depends_on: Vec<String>,
    pub slot_kind: Option<SlotKind>,
    pub update_all: bool,
    pub config: V,
}

impl Plan {
    pub fn affected(&self, changed: &[String]) -> bool {
        self.update_all || self.depends_on.iter().any(|code| changed.contains(code))
    }

    pub fn event(&self, status: &V, changed: &[String], rules: &V) -> Result<Option<V>, String> {
        if self
            .depends_on
            .first()
            .is_some_and(|code| changed.contains(code))
        {
            let event = self.read(status, 0.0, rules)?["event"].clone();
            if truth(&event) {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }
    pub fn role(&self, k: &str) -> Option<&Role> {
        self.roles.get(k)
    }

    /// A role-validated DP command.
    pub fn cmd(&self, k: &str, v: &V) -> Result<V, EngineError> {
        let r = self
            .role(k)
            .ok_or_else(|| EngineError::Unsupported(format!("{k} not available")))?;
        Ok(json!({"code":r.code,"value":r.write(v).map_err(EngineError::InvalidValue)?}))
    }

    pub fn read(&self, st: &V, total: f64, rules: &V) -> Result<V, String> {
        entity::state(self, st, total, rules)
    }

    pub fn write(&self, action: &str, args: &V, st: &V, rules: &V) -> Result<Vec<V>, EngineError> {
        entity::act(self, action, args, st, rules)
    }
}

/// A counted packet adds a delta once per timestamp; repeated values with new timestamps count.
pub(crate) fn accumulate(total: &mut f64, last_ts: &mut V, raw: &V, ts: &V, counted: bool) -> bool {
    if !counted || raw.is_null() || ts.is_null() || ts == last_ts {
        return false;
    }
    *total += n(raw);
    *last_ts = ts.clone();
    true
}
