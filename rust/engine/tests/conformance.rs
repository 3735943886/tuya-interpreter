use serde_json::{Value, json};
use tuya_interpreter::evaluate_json;

#[test]
fn rejects_invalid_requests() {
    for input in [
        "not json".to_string(),
        json!({"value": 1}).to_string(),
        json!({"op": "python_eval"}).to_string(),
    ] {
        let response: Value = serde_json::from_str(&evaluate_json(&input)).unwrap();
        assert_eq!(response["ok"], false);
    }
}

fn canonical(v: &Value) -> Value {
    match v {
        Value::Array(xs) => Value::Array(xs.iter().map(canonical).collect()),
        Value::Object(o) => {
            let mut o = o.clone();
            if o.get("type").is_some_and(|v| v == "reject") {
                o.remove("reason");
            }
            Value::Object(o)
        }
        _ => v.clone(),
    }
}

#[test]
fn frozen_driver_traces() {
    let rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    let cases: Vec<Value> = serde_json::from_str(include_str!("driver_vectors.json")).unwrap();
    let install: Value = serde_json::from_str(&evaluate_json(
        &json!({"op":"install_rules","rules_id":"conformance","rules":rules}).to_string(),
    ))
    .unwrap();
    assert_eq!(install["ok"], true);
    for case in cases {
        let call = |request: Value| {
            let mut request = request;
            request["rules_id"] = json!("conformance");
            let response: Value =
                serde_json::from_str(&evaluate_json(&request.to_string())).unwrap();
            assert_eq!(response["ok"], true, "{}: {response}", case["name"]);
            response["value"].clone()
        };
        let mut driver = call(
            json!({"op":"create","device":case["device"],"options":case["options"]}),
        )["driver"]
            .clone();
        assert_eq!(
            driver["assembly"]["descriptor"], case["descriptor"],
            "{} descriptor",
            case["name"]
        );
        for step in case["steps"].as_array().unwrap() {
            let response = call(
                json!({"op":"handle","driver":driver,"input":step["input"],"now":step["now"]}),
            );
            assert_eq!(
                canonical(&response["outputs"]),
                canonical(&step["outputs"]),
                "{} step {}: {}",
                case["name"],
                step["now"],
                step["input"]
            );
            driver = response["driver"].clone();
        }
    }
}

#[test]
fn ffi_ownership_and_invalid_inputs() {
    use std::ffi::CStr;
    use tuya_interpreter::{tuya_engine_eval, tuya_engine_free};
    for input in [std::ptr::null(), c"not json".as_ptr(), c"\xff".as_ptr()] {
        unsafe {
            let output = tuya_engine_eval(input);
            assert!(!output.is_null());
            let response: Value =
                serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
            assert_eq!(response["ok"], false);
            tuya_engine_free(output);
        }
    }
    unsafe {
        tuya_engine_free(std::ptr::null_mut());
    }
}

#[test]
fn host_allocated_requests_round_trip() {
    use std::ffi::CStr;
    use tuya_interpreter::{
        tuya_engine_alloc, tuya_engine_dealloc, tuya_engine_eval, tuya_engine_free,
    };
    let request = br#"{"op":"merge","mappings":[{"value":50}]}"#;
    let len = request.len() + 1;
    let buffer = tuya_engine_alloc(len);
    assert!(tuya_engine_alloc(0).is_null());
    unsafe {
        std::ptr::copy_nonoverlapping(request.as_ptr(), buffer, request.len());
        let output = tuya_engine_eval(buffer.cast());
        let response: Value = serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
        assert_eq!(response, json!({"ok": true, "value": {"value":50}}));
        tuya_engine_free(output);
        tuya_engine_dealloc(buffer, len);
        tuya_engine_dealloc(std::ptr::null_mut(), 0);
    }
}

fn call(request: Value) -> Value {
    serde_json::from_str(&evaluate_json(&request.to_string())).unwrap()
}

#[test]
fn drivers_require_valid_matching_ordered_rules() {
    let rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    let device = json!({"id":"rule-test"});
    for context in [
        json!({}),
        json!({"rules_id":"missing"}),
        json!({"rules":{"version":1}}),
    ] {
        let mut request = json!({"op":"create","device":device,"options":{}});
        request
            .as_object_mut()
            .unwrap()
            .extend(context.as_object().unwrap().clone());
        assert_eq!(call(request)["ok"], false);
    }
    assert_eq!(
        call(json!({"op":"install_rules","rules_id":"rule-test","rules":rules}))["ok"],
        true
    );
    let driver =
        call(json!({"op":"create","device":device,"rules_id":"rule-test"}))["value"]["driver"]
            .clone();
    let mut reversed = rules.clone();
    reversed["platforms"] = Value::Object(
        rules["platforms"]
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    assert_eq!(
        call(json!({"op":"install_rules","rules_id":"rule-test","rules":reversed}))["ok"],
        false
    );
    assert_eq!(
        call(json!({"op":"install_rules","rules_id":"rule-test-reversed","rules":reversed}))["ok"],
        true
    );
    for context in [
        json!({}),
        json!({"rules_id":"missing"}),
        json!({"rules_id":"rule-test-reversed"}),
        json!({"rules":reversed}),
    ] {
        let mut request =
            json!({"op":"handle","driver":driver,"input":{"type":"connected"},"now":0});
        request
            .as_object_mut()
            .unwrap()
            .extend(context.as_object().unwrap().clone());
        assert_eq!(call(request)["ok"], false);
    }
    assert_eq!(
        call(
            json!({"op":"handle","driver":driver,"input":{"type":"connected"},"now":0,"rules":rules})
        )["ok"],
        true
    );
}

#[test]
fn malformed_codec_inputs_return_errors_without_panicking() {
    let rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    let scene = format!("{}{}", "a".repeat(27), "é");
    let response = call(
        json!({"op":"strategy_read","name":"dj_v2_scene_alg","value":scene,"config":{},"rules":rules}),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"], "invalid scene");
    let response = call(
        json!({"op":"strategy_write","name":"cz_timer1_alg","value":[
        {"timer_switch":true,"week_day":[1],"start_time":"65535:00","end_time":"00:00"}
    ],"config":{}}),
    );
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"], "invalid time");
}

#[test]
fn cover_features_and_device_identity_follow_rules() {
    let mut rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    rules["features"]["cover"]["OPEN"] = json!(1024);
    rules["features"]["cover"]["CLOSE"] = json!(2048);
    rules["constants"]["manufacturer"] = json!("Custom vendor");
    rules["platforms"]["cover"]["categories"]["zz"] = json!([{"key":"control"}]);
    let device = json!({"id":"test","category":"zz","product_name":"Model","product_id":"test",
        "function":{"control":{"type":"Boolean","values":{}}}});
    let plans =
        call(json!({"op":"classify","device":device,"env":{},"platforms":["cover"],"rules":rules}));
    assert_eq!(plans["ok"], true, "{plans}");
    assert_eq!(plans["value"][0]["identity"]["supported_features"], 3072);
    let created = call(
        json!({"op":"create","device":device,"options":{"allow_hazardous":true},"rules":rules}),
    );
    assert_eq!(created["ok"], true, "{created}");
    let descriptor = &created["value"]["driver"]["assembly"]["descriptor"];
    assert!(descriptor["props"].get("open").is_some());
    assert_eq!(descriptor["vendor"], "Custom vendor");
}

#[test]
fn motion_property_comes_from_rules() {
    let mut rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    rules["motion_property"]["label"] = json!("Custom motion");
    let device = json!({"id":"test","category":"cl","product_id":"test",
        "function":{"control":{"type":"Enum","values":{"range":["open","stop","close"]}},
                    "percent_control":{"type":"Integer","values":{"min":0,"max":100,"scale":0,"step":1}}},
        "status_range":{"percent_state":{"type":"Integer","values":{"min":0,"max":100,"scale":0,"step":1}}}});
    let created = call(json!({"op":"create","device":device,
        "options":{"overrides":{"test":{"cover":{"state_source":"inferred","settle":4}}}},"rules":rules}));
    assert_eq!(created["ok"], true, "{created}");
    let driver = &created["value"]["driver"];
    assert_eq!(driver["motions"][0]["command"], "control");
    assert_eq!(driver["motions"][0]["settle"], 4.0);
    assert_eq!(
        driver["assembly"]["descriptor"]["props"]["cover_state"]["label"],
        "Custom motion"
    );
}

#[test]
fn vacuum_return_mode_is_configurable() {
    let mut rules: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    rules["constants"]["vacuum_return_mode"] = json!("custom_return");
    rules["platforms"]["vacuum"]["categories"]["zz"] = json!({"key":""});
    let device = json!({"category":"zz","function":{"mode":{"type":"Enum","values":{"range":["custom_return"]}}}});
    let plans = call(
        json!({"op":"classify","device":device,"env":{},"platforms":["vacuum"],"rules":rules}),
    );
    assert_eq!(plans["ok"], true, "{plans}");
    let plan = &plans["value"][0];
    assert_eq!(
        plan["identity"]["supported_features"].as_i64().unwrap() & 16,
        16
    );
    let written = call(
        json!({"op":"write","plan":plan,"action":"return_to_base","args":{},"status":{},"rules":rules}),
    );
    assert_eq!(
        written["value"],
        json!([{"code":"mode","value":"custom_return"}])
    );
}
