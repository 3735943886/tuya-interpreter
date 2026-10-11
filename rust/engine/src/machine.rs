//! Declarative converters: state-transition programs in the shared rule language (`expr`).
//! Programs are data: no eval, filesystem, networking, imports or clock reads.
use crate::data::*;
use crate::expr::{Effects, Flow, Scope, execute, target};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as V, json};
#[derive(Clone, Serialize, Deserialize)]
pub struct Machine {
    pub config: V,
    pub state: V,
}
#[derive(Default, Serialize)]
struct Output {
    values: Map<String, V>,
    timers: Map<String, V>,
    commands: Vec<V>,
}
impl Effects for Output {
    fn exec(&mut self, stmt: &V, scope: &mut Scope) -> Result<Flow, String> {
        if let Some(path) = stmt.get("set") {
            let parts = target(path);
            if parts.first().is_none_or(|p| p != "state") {
                return Err("rules can assign only state slots".into());
            }
            let value = scope.eval(&stmt["value"])?;
            let state = scope.get_mut("state").ok_or("no state")?;
            set(state, &parts[1..], value);
        } else if let Some(prop) = stmt.get("emit") {
            let value = scope.eval(&stmt["value"])?;
            self.values.insert(s(prop).into(), value);
        } else if let Some(name) = stmt.get("timer") {
            let after = scope.eval(&stmt["after"])?;
            self.timers.insert(s(name).into(), after);
        } else if let Some(send) = stmt.get("send") {
            let code = scope.eval(&send["code"])?;
            let value = scope.eval(&send["value"])?;
            self.commands.push(json!({"code":code,"value":value}));
        } else {
            return Err("unknown rule statement".into());
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
    pub fn invoke(&mut self, method: &str, input: &V) -> Result<V, String> {
        let program = if method == "write" {
            &self.config["write"][s(&input["prop"])]
        } else {
            &self.config[method]
        };
        let mut scope = Scope::new(&V::Null)
            .with_fields(input)
            .with("config", &self.config)
            .with("state", self.state.clone());
        let mut result = Output::default();
        execute(program, &mut scope, &mut result)?;
        let state = scope.take("state");
        let result = serde_json::to_value(result).map_err(|e| e.to_string())?;
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
            ]}),
            state: json!({"count":1}),
        };
        assert!(machine.invoke("update", &json!({})).is_err());
        assert_eq!(machine.state, json!({"count":1}));
    }

    #[test]
    fn nested_programs_share_instruction_budget() {
        let statement = json!({"set":["state","count"],"value":2});
        let mut machine = Machine {
            config: json!({"update":[{"if":true,"then":vec![statement; crate::expr::STATEMENT_LIMIT]}]}),
            state: json!({"count":1}),
        };
        assert_eq!(
            machine.invoke("update", &json!({})).unwrap_err(),
            "rule instruction limit exceeded"
        );
        assert_eq!(machine.state, json!({"count":1}));
    }
}
