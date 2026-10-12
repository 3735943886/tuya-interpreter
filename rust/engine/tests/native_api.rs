use serde_json::{Value, json};
use std::sync::Arc;
use tuya_interpreter::{
    Channel, Driver, DriverOptions, EngineError, Input, Output, RulePack, merge_rules,
};

fn pack() -> Arc<RulePack> {
    Arc::new(
        RulePack::new(
            serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap(),
        )
        .unwrap(),
    )
}

fn device() -> Value {
    json!({"id":"plug", "product_id":"p", "category":"kg",
        "function":{"switch_1":{"type":"Boolean","values":{}}}})
}

fn options() -> DriverOptions {
    DriverOptions {
        dpmap: json!({"1":"switch_1"}),
        ..DriverOptions::default()
    }
}

#[test]
fn packs_are_owned_by_callers_and_drivers_are_independent() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Driver>();
    let rules = pack();
    let weak = Arc::downgrade(&rules);
    let mut first = Driver::new(device(), rules.clone(), options()).unwrap();
    let second = Driver::new(device(), rules.clone(), options()).unwrap();
    drop(rules);
    first
        .handle(
            0.0,
            Input::Message {
                channel: Channel::State,
                json: json!({"1":true}),
            },
        )
        .unwrap();
    assert_eq!(first.values()["switch_1"], true);
    assert!(!second.synced());
    assert!(weak.upgrade().is_some());
    drop(first);
    drop(second);
    assert!(weak.upgrade().is_none());
}

#[test]
fn failure_does_not_commit_and_snapshots_check_rules() {
    let mut opts = options();
    opts.overrides = json!({"p":{"converters":{"declarative":{
        "props":{"count":{"type":"number"}}, "initial_state":{"count":0},
        "update":[{"set":"state.count","value":1},{"emit":"count","value":{"op":"unknown"}}]
    }}}});
    let rules = pack();
    let mut driver = Driver::new(device(), rules.clone(), opts).unwrap();
    driver.handle(0.0, Input::Connected).unwrap();
    let before = driver.snapshot();
    assert!(matches!(
        driver.handle(
            1.0,
            Input::Message {
                channel: Channel::Active,
                json: json!({"1":true})
            }
        ),
        Err(EngineError::Evaluation(_))
    ));
    assert_eq!(driver.snapshot(), before);
    assert!(matches!(
        driver.handle(f64::NAN, Input::Connected),
        Err(EngineError::InvalidInput(_))
    ));
    assert_eq!(
        Driver::restore(before.clone(), rules.clone())
            .unwrap()
            .snapshot(),
        before
    );
    let mut other = rules.rules().clone();
    other["constants"]["manufacturer"] = json!("Different");
    assert!(matches!(
        Driver::restore(before, Arc::new(RulePack::new(other).unwrap())),
        Err(EngineError::InvalidRules(_))
    ));
}

fn canonical(mut values: Value) -> Value {
    for value in values.as_array_mut().unwrap() {
        if value["type"] == "reject" {
            value.as_object_mut().unwrap().remove("reason");
        }
    }
    values
}

#[test]
fn native_api_replays_all_frozen_traces_without_json_transport() {
    let rules = pack();
    let cases: Vec<Value> = serde_json::from_str(include_str!("driver_vectors.json")).unwrap();
    assert_eq!(cases.len(), 324);
    for case in cases {
        let options = serde_json::from_value(case["options"].clone()).unwrap();
        let mut driver = Driver::new(case["device"].clone(), rules.clone(), options).unwrap();
        assert_eq!(driver.descriptor(), &case["descriptor"], "{}", case["name"]);
        for step in case["steps"].as_array().unwrap() {
            let input = serde_json::from_value(step["input"].clone()).unwrap();
            let outputs = driver.handle(step["now"].as_f64().unwrap(), input).unwrap();
            assert_eq!(
                canonical(json!(outputs)),
                canonical(step["outputs"].clone()),
                "{} at {}",
                case["name"],
                step["now"]
            );
        }
    }
}

#[test]
fn native_commands_use_the_same_effects_as_json() {
    let mut driver = Driver::new(device(), pack(), options()).unwrap();
    driver.handle(0.0, Input::Connected).unwrap();
    let output = driver
        .handle(
            1.0,
            Input::Command {
                prop: "switch_1".into(),
                value: json!(false),
            },
        )
        .unwrap();
    assert!(
        matches!(&output[..], [Output::SendMessage { json, .. }] if json == &json!({"dps":{"1":false}}))
    );
}

#[test]
fn native_rule_composition_reports_bad_layers_and_preserves_order() {
    let rules = pack();
    let composition = merge_rules(Some(&rules), vec![
        json!({"source":"custom", "rules":{"overrides":{"p":{"device":{"label":"Custom plug"}}}}}),
        json!({"source":"bad", "rules":{"overrides":{"p":{"dp":[]}}}}),
    ], true).unwrap();
    assert_eq!(composition.sources, ["custom"]);
    assert_eq!(composition.warnings.len(), 1);
    let driver = Driver::new(device(), Arc::new(composition.pack), options()).unwrap();
    assert_eq!(driver.descriptor()["label"], "Custom plug");
}

#[test]
fn invalid_snapshots_and_recursive_inputs_return_errors() {
    let rules = pack();
    let mut driver = Driver::new(device(), rules.clone(), options()).unwrap();
    let before = driver.snapshot();
    let mut bad = before.clone();
    bad["values"] = json!([]);
    assert!(matches!(
        Driver::restore(bad, rules),
        Err(EngineError::InvalidInput(_))
    ));
    let mut input = Input::Connected;
    for _ in 0..65 {
        input = Input::Describe { seed: vec![input] };
    }
    assert!(matches!(
        driver.handle(0.0, input),
        Err(EngineError::InvalidInput(_))
    ));
    assert_eq!(driver.snapshot(), before);
}
