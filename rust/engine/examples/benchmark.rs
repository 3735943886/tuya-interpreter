//! Same-process comparison of the typed Rust API and the JSON transport, excluding construction.
use serde_json::{Value, json};
use std::{hint::black_box, sync::Arc, time::Instant};
use tuya_rule_engine::{Channel, Driver, DriverOptions, Input, RulePack, evaluate_json};

fn call(request: Value) -> Value {
    let mut response: Value = serde_json::from_str(&evaluate_json(&request.to_string())).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    response["value"].take()
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    let iterations: usize = std::env::args()
        .nth(1)
        .map(|n| n.parse().unwrap())
        .unwrap_or(5000);
    assert!(iterations > 0);
    let pack: Value =
        serde_json::from_str(include_str!("../../../rules/builtin/00-default.json")).unwrap();
    let rules = Arc::new(RulePack::new(pack.clone()).unwrap());
    call(json!({"op":"install_rules","rules_id":"bench","rules":pack}));
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../tests/driver_vectors.json")).unwrap();
    let light = cases
        .iter()
        .find(|case| case["name"] == "dj_0gyaslysqfp4gfis")
        .unwrap();
    let switch = json!({"name":"switch", "device":{"id":"bench", "category":"kg",
        "function":{"switch_1":{"type":"Boolean","values":{}}}},
        "options":{"dpmap":{"1":"switch_1"}}});
    for case in [&switch, light] {
        let mut native_times = Vec::new();
        let mut json_times = Vec::new();
        for _ in 0..5 {
            let options: DriverOptions = serde_json::from_value(case["options"].clone()).unwrap();
            let mut native = Driver::new(case["device"].clone(), rules.clone(), options).unwrap();
            let mut wire = call(json!({"op":"create","rules_id":"bench","device":case["device"],"options":case["options"]}))["driver"].take();
            let seed = case["steps"]
                .as_array()
                .and_then(|steps| steps.iter().find(|step| step["input"]["type"] == "message"))
                .map(|step| step["input"].clone())
                .unwrap_or(json!({"type":"message","channel":"state","json":{"1":false}}));
            native
                .handle(0.0, serde_json::from_value(seed.clone()).unwrap())
                .unwrap();
            wire =
                call(json!({"op":"handle","rules_id":"bench","driver":wire,"input":seed,"now":0}))
                    ["driver"]
                    .take();
            for i in 0..100 {
                native
                    .handle(
                        i as f64,
                        Input::Message {
                            channel: Channel::Active,
                            json: json!({"1":i % 2 == 0}),
                        },
                    )
                    .unwrap();
                wire = call(json!({"op":"handle","rules_id":"bench","driver":wire,"input":{"type":"message","channel":"active","json":{"1":i % 2 == 0}},"now":i}))["driver"].take();
            }
            let start = Instant::now();
            for i in 0..iterations {
                black_box(
                    native
                        .handle(
                            i as f64,
                            Input::Message {
                                channel: Channel::Active,
                                json: json!({"1":i % 2 == 0}),
                            },
                        )
                        .unwrap(),
                );
            }
            native_times.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
            let start = Instant::now();
            for i in 0..iterations {
                let mut response = call(
                    json!({"op":"handle","rules_id":"bench","driver":wire,"input":{"type":"message","channel":"active","json":{"1":i % 2 == 0}},"now":i}),
                );
                black_box(&response["outputs"]);
                wire = response["driver"].take();
            }
            json_times.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
        }
        let native = median(native_times);
        let transport = median(json_times);
        println!(
            "{}",
            json!({"case":case["name"],"iterations":iterations,"rounds":5,
            "native_us":native,"json_us":transport,"speedup":transport/native})
        );
    }
}
