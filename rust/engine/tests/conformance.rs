use serde_json::{Value, json};
use tuya_rule_engine::evaluate_json;

#[test]
fn python_numeric_vectors() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("numeric_vectors.json")).unwrap();
    for case in cases {
        let actual: Value =
            serde_json::from_str(&evaluate_json(&case["request"].to_string())).unwrap();
        assert_eq!(
            actual["ok"], case["expected"]["ok"],
            "{}: {actual}",
            case["name"]
        );
        if actual["ok"] == true {
            let expected = &case["expected"]["value"];
            if expected.is_number() {
                assert_eq!(
                    actual["value"].as_f64(),
                    expected.as_f64(),
                    "{}",
                    case["name"]
                );
            } else if expected.is_boolean() {
                // Python integer validation preserves bool; later scale converts it to numeric.
                assert_eq!(
                    actual["value"].as_f64(),
                    Some(if expected == true { 1.0 } else { 0.0 })
                );
            } else {
                assert_eq!(&actual["value"], expected, "{}", case["name"]);
            }
        }
    }
}

#[test]
fn rejects_invalid_protocol() {
    for input in [
        "not json".to_string(),
        json!({"version": 99, "value": 1, "ops": []}).to_string(),
        json!({"version": 1, "value": 1, "ops": [{"op": "python_eval"}]}).to_string(),
        json!({"version": 1, "value": 1, "ops": [], "unexpected": true}).to_string(),
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
fn full_driver_reference_traces() {
    let rules: Value =
        serde_json::from_str(include_str!("../../../rules/00-default.json")).unwrap();
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
    use tuya_rule_engine::{tuya_engine_eval, tuya_engine_free};
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
    use tuya_rule_engine::{
        tuya_engine_alloc, tuya_engine_dealloc, tuya_engine_eval, tuya_engine_free,
    };
    let request = br#"{"op":"il","action":"encode","value":50.0}"#;
    let len = request.len() + 1;
    let buffer = tuya_engine_alloc(len);
    assert!(tuya_engine_alloc(0).is_null());
    unsafe {
        std::ptr::copy_nonoverlapping(request.as_ptr(), buffer, request.len());
        let output = tuya_engine_eval(buffer.cast());
        let response: Value = serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
        assert_eq!(response, json!({"ok": true, "value": "50"}));
        tuya_engine_free(output);
        tuya_engine_dealloc(buffer, len);
        tuya_engine_dealloc(std::ptr::null_mut(), 0);
    }
}

fn call(request: Value) -> Value {
    serde_json::from_str(&evaluate_json(&request.to_string())).unwrap()
}

#[test]
fn malformed_codec_inputs_return_errors_without_panicking() {
    let scene = format!("{}{}", "a".repeat(27), "é");
    let response =
        call(json!({"op":"strategy_read","name":"dj_v2_scene_alg","value":scene,"config":{}}));
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
        serde_json::from_str(include_str!("../../../rules/00-default.json")).unwrap();
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
fn motion_configuration_comes_from_rules() {
    let mut rules: Value =
        serde_json::from_str(include_str!("../../../rules/00-default.json")).unwrap();
    rules["motion_defaults"]["command"] = json!("custom_command");
    rules["motion_defaults"]["settle"] = json!(4);
    rules["motion_property"]["label"] = json!("Custom motion");
    let created = call(
        json!({"op":"create","device":{"id":"test","product_id":"test"},
        "options":{"overrides":{"test":{"auto":false,"converters":{"cover_motion":{}}}}},"rules":rules}),
    );
    assert_eq!(created["ok"], true, "{created}");
    let driver = &created["value"]["driver"];
    assert_eq!(driver["motions"][0]["command"], "custom_command");
    assert_eq!(driver["motions"][0]["settle"], 4.0);
    assert_eq!(
        driver["assembly"]["descriptor"]["props"]["cover_state"]["label"],
        "Custom motion"
    );
}

#[test]
fn vacuum_return_mode_is_configurable() {
    let mut rules: Value =
        serde_json::from_str(include_str!("../../../rules/00-default.json")).unwrap();
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
