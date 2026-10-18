//! Declarative converters: state-transition programs in the shared rule language (`expr`).
//! Programs are data: no eval, filesystem, networking, imports or clock reads.
use crate::EngineError;
use crate::data::*;
use crate::error::OrEval;
use crate::expr::{Effects, Flow, Scope, execute, target};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as V, json};
use std::sync::Arc;
#[derive(Clone, Serialize, Deserialize)]
pub struct Machine {
    pub config: Arc<V>,
    pub state: V,
}
pub(crate) enum Method {
    Update,
    Timer,
    Reset,
    Write,
}
#[derive(Debug, Default, Serialize)]
pub(crate) struct MachineEffects {
    pub values: Map<String, V>,
    pub timers: IndexMap<String, Option<f64>>,
    pub commands: Vec<V>,
}
impl Effects for MachineEffects {
    fn exec(&mut self, stmt: &V, scope: &mut Scope) -> Result<Flow, EngineError> {
        if let Some(path) = stmt.get("set") {
            let parts = target(path);
            if parts.first().is_none_or(|p| p != "state") {
                return Err(EngineError::eval("rules can assign only state slots"));
            }
            let value = scope.eval(&stmt["value"]).or_eval()?;
            let state = scope.get_mut("state").ok_or("no state").or_eval()?;
            set(state, &parts[1..], value);
        } else if let Some(prop) = stmt.get("emit") {
            let value = scope.eval(&stmt["value"]).or_eval()?;
            self.values.insert(s(prop).into(), value);
        } else if let Some(name) = stmt.get("timer") {
            let after = scope.eval(&stmt["after"]).or_eval()?;
            let after = if after.is_null() {
                None
            } else {
                Some(
                    after
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .ok_or("timer delay must be a finite nonnegative number or null")
                        .or_eval()?,
                )
            };
            self.timers.insert(s(name).into(), after);
        } else if let Some(send) = stmt.get("send") {
            let code = scope.eval(&send["code"]).or_eval()?;
            let value = scope.eval(&send["value"]).or_eval()?;
            self.commands.push(json!({"code":code,"value":value}));
        } else {
            return Err(EngineError::eval("unknown rule statement"));
        }
        Ok(Flow::Next)
    }
}
/// Assignment keeps explicit nulls: a converter's state slot may hold null.
fn set(root: &mut V, parts: &[String], value: V) {
    let mut v = root;
    for part in parts {
        if !v.is_object() {
            *v = json!({});
        }
        v = v
            .as_object_mut()
            .expect("object")
            .entry(part.clone())
            .or_insert(V::Null);
    }
    *v = value;
}
impl Machine {
    pub fn new(config: &V) -> Self {
        let config = Arc::new(config.clone());
        let state = config.get("initial_state").cloned().unwrap_or(json!({}));
        Self { config, state }
    }
    /// Run the `reset` program; if it fails, fall back to the initial state so a disconnect always lands.
    pub fn reset(&mut self) {
        if self.invoke(Method::Reset, &json!({})).is_err() {
            self.state = self
                .config
                .get("initial_state")
                .cloned()
                .unwrap_or(json!({}));
        }
    }
    pub fn invoke(&mut self, method: Method, input: &V) -> Result<MachineEffects, EngineError> {
        let program = match method {
            Method::Write => &self.config["write"][s(&input["prop"])],
            Method::Update => &self.config["update"],
            Method::Timer => &self.config["timer"],
            Method::Reset => &self.config["reset"],
        };
        let mut scope = Scope::new(&V::Null)
            .with_fields(input)
            .with("config", self.config.as_ref())
            .with("state", self.state.clone());
        let mut result = MachineEffects::default();
        execute(program, &mut scope, &mut result)?;
        let state = scope.take("state");
        for (prop, value) in &mut result.values {
            let definition = self.config["props"].get(prop).ok_or_else(|| {
                EngineError::Evaluation(format!("converter emits undeclared property {prop}"))
            })?;
            *value = crate::assembly::il_value(value.take(), definition);
            if !value.is_null() && !crate::driver::overrides::value_fits(value, definition) {
                return Err(EngineError::eval(format!(
                    "converter value does not fit property {prop}"
                )));
            }
        }
        self.state = state;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_program_does_not_commit_state() {
        let mut machine = Machine {
            config: json!({"update":[
                {"set":["state","count"],"value":2},
                {"emit":"count","value":{"op":"unknown"}}
            ]})
            .into(),
            state: json!({"count":1}),
        };
        assert!(machine.invoke(Method::Update, &json!({})).is_err());
        assert_eq!(machine.state, json!({"count":1}));
    }

    #[test]
    fn nested_programs_share_instruction_budget() {
        let statement = json!({"set":["state","count"],"value":2});
        let mut machine = Machine {
            config:
                json!({"update":[{"if":true,"then":vec![statement; crate::expr::STATEMENT_LIMIT]}]})
                    .into(),
            state: json!({"count":1}),
        };
        assert_eq!(
            machine.invoke(Method::Update, &json!({})).unwrap_err(),
            EngineError::Evaluation("rule instruction limit exceeded".into())
        );
        assert_eq!(machine.state, json!({"count":1}));
    }
}
