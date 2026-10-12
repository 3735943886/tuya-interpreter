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

#[test]
fn engine_held_drivers_keep_state_between_requests() {
    let call = |request: Value| -> Value {
        serde_json::from_str(&tuya_interpreter::evaluate_json(&request.to_string())).unwrap()
    };
    let rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    let opened = |rules: &Value| {
        call(json!({"op":"open","rules":rules,"device":device(),"options":{"dpmap":{"1":"switch_1"}}}))
            ["value"]["handle"]
            .as_u64()
            .unwrap()
    };
    let handle = opened(&rules);
    let drive = |handle: u64, now: Value, input: Value| {
        call(json!({"op":"drive","handle":handle,"now":now,"input":input}))
    };
    let linked = drive(handle, json!(0), json!({"type":"connected"}));
    assert_eq!(linked["value"]["linked"], true);
    let on = drive(
        handle,
        json!(1),
        json!({"type":"message","channel":"active","json":{"1":true}}),
    );
    assert!(
        on["value"]["outputs"]
            .as_array()
            .unwrap()
            .contains(&json!({"type":"value","prop":"switch_1","value":true}))
    );
    // The same message again changes nothing: the value was kept in the engine.
    let again = drive(
        handle,
        json!(2),
        json!({"type":"message","channel":"active","json":{"1":true}}),
    );
    assert_eq!(again["value"]["outputs"], json!([]));
    let view = call(json!({"op":"view","handle":handle}))["value"].clone();
    assert_eq!(view["dps"], json!({"1":true}));
    assert_eq!(view["descriptor"]["props"]["switch_1"]["type"], "binary");
    let snapshot = call(json!({"op":"snapshot","handle":handle}))["value"].clone();
    assert_eq!(snapshot["values"]["switch_1"], true);

    // A failed input leaves the driver unchanged.
    assert_eq!(
        drive(handle, json!(null), json!({"type":"connected"}))["ok"],
        false
    );
    assert_eq!(
        call(json!({"op":"snapshot","handle":handle}))["value"],
        snapshot
    );

    let other = opened(&rules);
    assert_eq!(
        call(json!({"op":"carry_handle","handle":other,"old":handle}))["value"],
        true
    );
    assert_eq!(
        call(json!({"op":"carry_handle","handle":handle,"old":handle}))["value"],
        true
    );
    for h in [handle, other] {
        assert_eq!(call(json!({"op":"close","handle":h}))["value"], true);
        let closed = drive(h, json!(3), json!({"type":"connected"}));
        assert_eq!(closed["ok"], false);
        assert!(closed["error"].as_str().unwrap().contains("no open driver"));
    }
    assert_eq!(call(json!({"op":"view","handle":"x"}))["ok"], false);
}
