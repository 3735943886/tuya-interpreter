use crate::data::*;
use crate::entity;
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
    pub slot_kind: Option<String>,
    pub update_all: bool,
    pub config: V,
}

impl Plan {
    pub fn role(&self, k: &str) -> Option<&Role> {
        self.roles.get(k)
    }

    /// A role-validated DP command.
    pub fn cmd(&self, k: &str, v: &V) -> Result<V, String> {
        let r = self
            .role(k)
            .ok_or_else(|| format!("unsupported: {k} not available"))?;
        Ok(json!({"code":r.code,"value":r.write(v)?}))
    }

    pub fn read(&self, st: &V, total: f64, rules: &V) -> Result<V, String> {
        entity::state(self, st, total, rules)
    }

    pub fn write(&self, action: &str, args: &V, st: &V, rules: &V) -> Result<Vec<V>, String> {
        entity::act(self, action, args, st, rules)
    }
}
