//! In-memory Rust API. Packs belong to callers, never to a process-global registry.
use crate::{EngineError, Input, Output};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug)]
pub struct RulePack {
    rules: Value,
    fingerprint: String,
}

impl RulePack {
    pub fn new(rules: Value) -> Result<Self, EngineError> {
        if rules["version"] != 1 {
            return Err(EngineError::InvalidRules("unsupported rule version".into()));
        }
        crate::rules::validate(&rules).map_err(EngineError::InvalidRules)?;
        let fingerprint = fingerprint(&rules);
        Ok(Self { rules, fingerprint })
    }
    pub fn rules(&self) -> &Value {
        &self.rules
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

pub struct RuleComposition {
    pub pack: RulePack,
    pub sources: Vec<String>,
    pub warnings: Vec<String>,
}

/// Compose host-decoded layers in order. Invalid layers can be skipped with explicit warnings.
pub fn merge_rules(
    base: Option<&RulePack>,
    layers: Vec<Value>,
    tolerant: bool,
) -> Result<RuleComposition, EngineError> {
    let mut request = json!({"layers":layers,"tolerant":tolerant});
    if let Some(base) = base {
        request["base"] = base.rules.clone();
    }
    let mut result = crate::rules::compose(&request).map_err(EngineError::InvalidRules)?;
    Ok(RuleComposition {
        pack: RulePack::new(result["rules"].take())?,
        sources: serde_json::from_value(result["sources"].take())
            .map_err(|e| EngineError::Evaluation(e.to_string()))?,
        warnings: serde_json::from_value(result["warnings"].take())
            .map_err(|e| EngineError::Evaluation(e.to_string()))?,
    })
}

pub(crate) fn fingerprint(rules: &Value) -> String {
    format!("{:x}", Sha256::digest(rules.to_string().as_bytes()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HostEnv {
    pub temperature_unit: String,
    pub allowed_units: Value,
}
impl Default for HostEnv {
    fn default() -> Self {
        Self {
            temperature_unit: "C".into(),
            allowed_units: json!({}),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DriverOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HostEnv>,
    pub use_quirks: bool,
    pub dpmap: Value,
    pub allow_hazardous: bool,
    pub expose_unused: bool,
    pub overrides: Value,
    pub device_settings: bool,
}
impl Default for DriverOptions {
    fn default() -> Self {
        Self {
            env: None,
            use_quirks: true,
            dpmap: json!({}),
            allow_hazardous: false,
            expose_unused: false,
            overrides: json!({}),
            device_settings: false,
        }
    }
}

pub struct Driver {
    state: crate::driver::Driver,
    rules: Arc<RulePack>,
}

impl Driver {
    pub fn new(
        device: Value,
        rules: Arc<RulePack>,
        options: DriverOptions,
    ) -> Result<Self, EngineError> {
        let options =
            serde_json::to_value(options).map_err(|e| EngineError::InvalidInput(e.to_string()))?;
        let mut state = crate::driver::Driver::new(&device, &options, rules.rules())?;
        state.rules_fingerprint = rules.fingerprint.clone();
        Ok(Self { state, rules })
    }
    /// A failed input leaves the driver unchanged. The caller owns time and executes all effects.
    pub fn handle(&mut self, now: f64, input: Input) -> Result<Vec<Output>, EngineError> {
        if !now.is_finite() {
            return Err(EngineError::InvalidInput(
                "now must be a finite number supplied by the host".into(),
            ));
        }
        let mut next = self.state.clone();
        let outputs = next.handle(now, &input, self.rules.rules())?;
        self.state = next;
        Ok(outputs)
    }
    pub fn descriptor(&self) -> &Value {
        &self.state.assembly.descriptor
    }
    pub fn values(&self) -> &Value {
        &self.state.values
    }
    pub fn dps(&self) -> &Value {
        &self.state.dps
    }
    pub fn linked(&self) -> bool {
        self.state.linked
    }
    pub fn synced(&self) -> bool {
        self.state.synced
    }
    pub fn timers(&self) -> &[String] {
        &self.state.timers
    }
    pub fn settings_block(&self) -> Value {
        self.state.settings_block()
    }
    pub fn carry(&mut self, old: &Self) {
        self.state.carry(&old.state);
    }
    /// Version-specific state for migration across processes, not a durable storage format.
    pub fn snapshot(&self) -> Value {
        json!(self.state)
    }
    pub fn restore(snapshot: Value, rules: Arc<RulePack>) -> Result<Self, EngineError> {
        let state: crate::driver::Driver = serde_json::from_value(snapshot)
            .map_err(|e| EngineError::InvalidInput(e.to_string()))?;
        if state.rules_fingerprint != rules.fingerprint {
            return Err(EngineError::InvalidRules(
                "driver rules do not match the supplied rule pack".into(),
            ));
        }
        state.validate_state()?;
        Ok(Self { state, rules })
    }
}
