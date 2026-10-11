mod adapter;
mod api;
mod assembly;
mod data;
mod driver;
mod entity;
mod expr;
mod machine;
mod plan;
mod rules;
// Versioned JSON rule engine. No device I/O or clock access.
use serde::Deserialize;
use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    version: u32,
    value: Value,
    ops: Vec<Op>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Op {
    ValidateInteger { min: i64, max: i64 },
    ValidateNumber,
    Scale { scale: i32 },
    Unscale { scale: i32 },
    Invert { max: f64 },
    RoundHalfEven,
    ValidateRange { min: f64, max: f64 },
}

fn numeric(value: &Value) -> Option<f64> {
    // Python's existing numeric ops intentionally accept bool as an integer.
    value
        .as_f64()
        .or_else(|| value.as_bool().map(|v| if v { 1.0 } else { 0.0 }))
}

pub fn evaluate(request: Request) -> Result<Value, String> {
    if request.version != 1 {
        return Err("unsupported rule version".into());
    }
    let mut value = request.value;
    for op in request.ops {
        value = match op {
            Op::ValidateInteger { min, max } => {
                if min > max {
                    return Err("invalid integer range".into());
                }
                let integer = value.is_i64() || value.is_u64() || value.is_boolean();
                match numeric(&value) {
                    Some(v) if integer && v >= min as f64 && v <= max as f64 => value,
                    _ => Value::Null,
                }
            }
            Op::ValidateNumber => {
                if numeric(&value).is_none() {
                    return Err("invalid numeric value".into());
                }
                value
            }
            Op::ValidateRange { min, max } => {
                if min > max {
                    return Err("invalid range".into());
                }
                match numeric(&value) {
                    Some(v) if v >= min && v <= max => value,
                    _ => return Err("value out of range".into()),
                }
            }
            other => {
                if value.is_null() {
                    continue;
                }
                let v = numeric(&value).ok_or("invalid numeric value")?;
                let result = match other {
                    Op::Scale { scale } => v / 10_f64.powi(scale),
                    Op::Unscale { scale } => v * 10_f64.powi(scale),
                    Op::Invert { max } => max - v,
                    Op::RoundHalfEven => v.round_ties_even(),
                    _ => unreachable!(),
                };
                if !result.is_finite() {
                    return Err("non-finite numeric result".into());
                }
                json!(result)
            }
        };
    }
    Ok(value)
}

pub fn evaluate_json(input: &str) -> String {
    let result = serde_json::from_str::<Value>(input)
        .map_err(|e| e.to_string())
        .and_then(|request| {
            if request.get("op").is_some() {
                api::request(&request)
            } else {
                serde_json::from_value::<Request>(request)
                    .map_err(|e| e.to_string())
                    .and_then(evaluate)
            }
        });
    match result {
        Ok(value) => json!({"ok": true, "value": value}).to_string(),
        Err(error) => json!({"ok": false, "error": error}).to_string(),
    }
}

/// Evaluate a UTF-8, NUL-terminated JSON request. Null returns an error response.
/// The returned allocation must be released with `tuya_engine_free`.
///
/// # Safety
/// A non-null input must point to a readable NUL-terminated byte string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tuya_engine_eval(input: *const c_char) -> *mut c_char {
    let response = std::panic::catch_unwind(|| {
        if input.is_null() {
            return json!({"ok": false, "error": "null input"}).to_string();
        }
        match unsafe { CStr::from_ptr(input) }.to_str() {
            Ok(text) => evaluate_json(text),
            Err(_) => json!({"ok": false, "error": "invalid UTF-8"}).to_string(),
        }
    })
    .unwrap_or_else(|_| json!({"ok": false, "error": "engine panic"}).to_string());
    CString::new(response).expect("JSON escapes NUL").into_raw()
}

/// # Safety
/// Pointer must be null or an unreleased allocation from `tuya_engine_eval`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tuya_engine_free(output: *mut c_char) {
    if !output.is_null() {
        drop(unsafe { CString::from_raw(output) });
    }
}

/// A zeroed buffer of `len` bytes in this library's memory, for a host that cannot pass its own
/// (WebAssembly): write the request and its terminating NUL there, then release it with
/// `tuya_engine_dealloc(buffer, len)`. Returns null for `len == 0`.
#[unsafe(no_mangle)]
pub extern "C" fn tuya_engine_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    Box::into_raw(vec![0u8; len].into_boxed_slice()).cast()
}

/// # Safety
/// `buffer` must be null or an unreleased `tuya_engine_alloc(len)` allocation of the same `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tuya_engine_dealloc(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buffer, len)) });
    }
}
