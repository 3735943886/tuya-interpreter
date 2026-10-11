//! Sans-I/O Tuya interpretation with an in-memory Rust API and a portable JSON C ABI.
//! Hosts supply rules, inputs and time, and execute returned effects. The core performs no I/O.
//! Use [`RulePack`] and [`Driver`] to keep typed state in memory; [`evaluate_json`] preserves
//! the explicit-state transport used by Python and WebAssembly.
mod adapter;
mod api;
mod assembly;
mod data;
mod driver;
mod entity;
mod error;
mod expr;
mod machine;
mod plan;
mod protocol;
mod rules;
mod runtime;
pub use error::EngineError;
pub use protocol::{
    Channel, DpKind, Input, MotionState, Output, RejectCode, SendChannel, StateSource,
};
pub use runtime::{Driver, DriverOptions, HostEnv, RuleComposition, RulePack, merge_rules};
// JSON rule engine behind a C ABI. No device I/O or clock access.
use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char};

pub fn evaluate_json(input: &str) -> String {
    let result = serde_json::from_str::<Value>(input)
        .map_err(|e| EngineError::InvalidInput(e.to_string()))
        .and_then(|request| api::request(&request));
    match result {
        Ok(value) => json!({"ok": true, "value": value}).to_string(),
        Err(error) => json!({"ok": false, "error": error.to_string()}).to_string(),
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
