use crate::error::HkError;
use crate::parser::{load_hk_file, parse_hk};
use crate::resolve::{get_value_by_path, resolve_interpolations};
use crate::serialize::serialize_hk;
use crate::value::{HkConfig, HkValue};

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

// ─── Global registry ───────────────────────────────────────────────────────

lazy_static! {
    static ref REGISTRY: Mutex<HashMap<i32, HkConfig>> = Mutex::new(HashMap::new());
    static ref LAST_ERROR: Mutex<String> = Mutex::new(String::new());
}

static NEXT_HANDLE: AtomicI32 = AtomicI32::new(1);

fn set_error(msg: String) {
    *LAST_ERROR.lock().unwrap() = msg;
}

fn clear_error() {
    LAST_ERROR.lock().unwrap().clear();
}

fn store_config(cfg: HkConfig) -> i32 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    REGISTRY.lock().unwrap().insert(handle, cfg);
    clear_error();
    handle
}

fn with_config<R>(handle: i32, f: impl FnOnce(&HkConfig) -> R) -> Option<R> {
    let reg = REGISTRY.lock().unwrap();
    reg.get(&handle).map(f)
}

fn with_config_mut<R>(handle: i32, f: impl FnOnce(&mut HkConfig) -> R) -> Option<R> {
    let mut reg = REGISTRY.lock().unwrap();
    reg.get_mut(&handle).map(f)
}

// ─── C-string marshaling helpers ────────────────────────────────────────────

/// `None` for a null pointer or invalid UTF-8 (lossily recovered instead
/// of trapping — a malformed path/key should surface as a normal "not
/// found"/parse error on the Rust side, not a native crash).
unsafe fn read_c_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

fn to_c_string(s: String) -> *mut c_char {
    // A NUL byte can never legally occur inside a parsed `.hk` string
    // value, but fall back to an empty string rather than panic if one
    // somehow does (e.g. pasted through an env var interpolation).
    CString::new(s).unwrap_or_default().into_raw()
}

fn err_to_string(e: HkError) -> String {
    e.to_string()
}

// ─── Parsing ────────────────────────────────────────────────────────────────

/// Parse `.hk` source text already in memory. Returns a handle (`0` on a
/// parse error — check `hk_last_error`).
#[no_mangle]
pub extern "C" fn hk_parse(text: *const c_char) -> i32 {
    let text = match unsafe { read_c_str(text) } {
        Some(t) => t,
        None => {
            set_error("hk_parse: null text pointer".to_string());
            return 0;
        }
    };
    match parse_hk(&text) {
        Ok(cfg) => store_config(cfg),
        Err(e) => {
            set_error(err_to_string(e));
            0
        }
    }
}

/// Parse a `.hk` file from disk. Same handle/error convention as `hk_parse`.
#[no_mangle]
pub extern "C" fn hk_parse_file(path: *const c_char) -> i32 {
    let path = match unsafe { read_c_str(path) } {
        Some(p) => p,
        None => {
            set_error("hk_parse_file: null path pointer".to_string());
            return 0;
        }
    };
    match load_hk_file(&path) {
        Ok(cfg) => store_config(cfg),
        Err(e) => {
            set_error(err_to_string(e));
            0
        }
    }
}

/// Expand `${env:VAR}` / `${section.key}` interpolations in place.
/// `1` on success, `0` on a cyclic/invalid reference or an absent handle.
#[no_mangle]
pub extern "C" fn hk_resolve(handle: i32) -> i32 {
    let result = with_config_mut(handle, |cfg| resolve_interpolations(cfg));
    match result {
        Some(Ok(())) => {
            clear_error();
            1
        }
        Some(Err(e)) => {
            set_error(err_to_string(e));
            0
        }
        None => {
            set_error(format!("hk_resolve: unknown handle {handle}"));
            0
        }
    }
}

/// Release a parsed document. Safe to call more than once, and safe to
/// call with a handle that's already been freed or was never valid.
#[no_mangle]
pub extern "C" fn hk_free(handle: i32) {
    if handle == 0 {
        return;
    }
    REGISTRY.lock().unwrap().remove(&handle);
}

// ─── Lookups (dotted-path, e.g. "metadata.authors[0]") ─────────────────────

#[no_mangle]
pub extern "C" fn hk_has(handle: i32, key: *const c_char) -> i32 {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return 0,
    };
    with_config(handle, |cfg| get_value_by_path(&key, cfg).is_some())
        .unwrap_or(false) as i32
}

#[no_mangle]
pub extern "C" fn hk_get_string(handle: i32, key: *const c_char) -> *mut c_char {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return to_c_string(String::new()),
    };
    let s = with_config(handle, |cfg| {
        get_value_by_path(&key, cfg).and_then(|v| v.as_string().ok())
    })
    .flatten()
    .unwrap_or_default();
    to_c_string(s)
}

#[no_mangle]
pub extern "C" fn hk_get_number(handle: i32, key: *const c_char) -> f64 {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return 0.0,
    };
    with_config(handle, |cfg| {
        get_value_by_path(&key, cfg).and_then(|v| v.as_number().ok())
    })
    .flatten()
    .unwrap_or(0.0)
}

#[no_mangle]
pub extern "C" fn hk_get_bool(handle: i32, key: *const c_char) -> i32 {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return 0,
    };
    with_config(handle, |cfg| {
        get_value_by_path(&key, cfg).and_then(|v| v.as_bool().ok())
    })
    .flatten()
    .unwrap_or(false) as i32
}

/// `"string"` | `"number"` | `"bool"` | `"array"` | `"map"` | `"none"`.
#[no_mangle]
pub extern "C" fn hk_type_of(handle: i32, key: *const c_char) -> *mut c_char {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return to_c_string("none".to_string()),
    };
    let ty = with_config(handle, |cfg| {
        get_value_by_path(&key, cfg).map(|v| match v {
            HkValue::String(_) => "string",
            HkValue::Number(_) => "number",
            HkValue::Bool(_) => "bool",
            HkValue::Array(_) => "array",
            HkValue::Map(_) => "map",
        })
    })
    .flatten()
    .unwrap_or("none");
    to_c_string(ty.to_string())
}

// ─── Arrays ─────────────────────────────────────────────────────────────────

/// Number of items in the array at `key`, or `-1` if `key` doesn't resolve
/// to an array (missing, wrong type, or an unknown handle).
#[no_mangle]
pub extern "C" fn hk_array_len(handle: i32, key: *const c_char) -> i32 {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return -1,
    };
    with_config(handle, |cfg| {
        get_value_by_path(&key, cfg).and_then(|v| v.as_array().ok()).map(|a| a.len() as i32)
    })
    .flatten()
    .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn hk_array_get_string(handle: i32, key: *const c_char, index: i32) -> *mut c_char {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return to_c_string(String::new()),
    };
    if index < 0 {
        return to_c_string(String::new());
    }
    let s = with_config(handle, |cfg| {
        get_value_by_path(&key, cfg)
            .and_then(|v| v.as_array().ok())
            .and_then(|a| a.get(index as usize))
            .and_then(|v| v.as_string().ok())
    })
    .flatten()
    .unwrap_or_default();
    to_c_string(s)
}

#[no_mangle]
pub extern "C" fn hk_array_get_number(handle: i32, key: *const c_char, index: i32) -> f64 {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return 0.0,
    };
    if index < 0 {
        return 0.0;
    }
    with_config(handle, |cfg| {
        get_value_by_path(&key, cfg)
            .and_then(|v| v.as_array().ok())
            .and_then(|a| a.get(index as usize))
            .and_then(|v| v.as_number().ok())
    })
    .flatten()
    .unwrap_or(0.0)
}

// ─── Map key enumeration ─────────────────────────────────────────────────────
//
// `main.h#`'s original accessor set (`has`/`get`/`array`/...) has no way to
// ask "what keys does this section/sub-map actually have" — every other
// accessor needs the caller to already know the key it wants. That's fine
// for `[package]`/`[build]` (fixed field names), but `bytes`'s own
// `Bytes.hk` has whole sections — `[deps]`, `[features]`, `[python]`,
// `[registry]`, `[workspace] -> languages` — whose *keys themselves* are
// the data (a dependency name, a feature name, ...), not just their
// values. `config::parse` (see `bytes`'s `src/config.h#`) needs to iterate
// those, so it needs real key enumeration — hence these two, added
// alongside (not replacing) the original array-style pair
// `hk_array_len`/`hk_array_get_string` they're deliberately modeled on.

/// Number of keys in the map at `key` (in file order — `HkConfig` is an
/// `IndexMap`), or `-1` if `key` doesn't resolve to a map. Pass `""` for
/// the document root (top-level section names).
#[no_mangle]
pub extern "C" fn hk_map_key_count(handle: i32, key: *const c_char) -> i32 {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return -1,
    };
    with_config(handle, |cfg| map_at(cfg, &key).map(|m| m.len() as i32))
        .flatten()
        .unwrap_or(-1)
}

/// The `index`-th key of the map at `key`, in file order, or `""` if out
/// of range / not a map / unknown handle.
#[no_mangle]
pub extern "C" fn hk_map_key_at(handle: i32, key: *const c_char, index: i32) -> *mut c_char {
    let key = match unsafe { read_c_str(key) } {
        Some(k) => k,
        None => return to_c_string(String::new()),
    };
    if index < 0 {
        return to_c_string(String::new());
    }
    let s = with_config(handle, |cfg| {
        map_at(cfg, &key).and_then(|m| m.get_index(index as usize)).map(|(k, _)| k.clone())
    })
    .flatten()
    .unwrap_or_default();
    to_c_string(s)
}

/// `""` means "the document root" (top-level section names, e.g.
/// `"package"`, `"deps"`, ...); anything else is a dotted path into it,
/// same convention every other accessor here already uses.
fn map_at<'a>(cfg: &'a HkConfig, key: &str) -> Option<&'a indexmap::IndexMap<String, HkValue>> {
    if key.is_empty() {
        return Some(cfg);
    }
    get_value_by_path(key, cfg).and_then(|v| v.as_map().ok())
}

// ─── Serialization / errors ─────────────────────────────────────────────────

/// Serialize the whole (possibly `resolve`d) document back to `.hk` text.
#[no_mangle]
pub extern "C" fn hk_serialize(handle: i32) -> *mut c_char {
    let s = with_config(handle, serialize_hk).unwrap_or_default();
    to_c_string(s)
}

/// The last error message recorded on the native side (`""` if none, or
/// once it's been superseded by a later successful call).
#[no_mangle]
pub extern "C" fn hk_last_error() -> *mut c_char {
    to_c_string(LAST_ERROR.lock().unwrap().clone())
}

/// Frees a string previously returned by any `hk_*` function above.
/// Not currently called from `main.h#`'s own H# side (its `extern dynamic
/// [c]` marshaling copies the C string into a managed H# string
/// immediately on return), but exported for any other native or FFI
/// caller of this `cdylib` that does need to release the buffer itself
/// — see this file's own module doc comment ("Memory / ownership").
#[no_mangle]
pub extern "C" fn hk_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(s));
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────
//
// Exercises the exact shape `bytes`'s own `src/config.h#` rewrite needs:
// fixed-field sections via dotted `hk_get_string` (`[package]`), and
// dynamic-key sections via `hk_map_key_count`/`hk_map_key_at`
// (`[deps]`/`[features]`), including a hyphenated key name and a
// trailing-space " optional" marker, both of which the old hand-rolled
// `hk_parse_key`/`hk_parse_value` used to special-case.
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    unsafe fn from_c(ptr: *mut c_char) -> String {
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        hk_free_string(ptr);
        s
    }

    const SAMPLE: &str = "\
[package]
-> name => bytes
-> version => 0.9

[deps]
-> hk-parser => bytes
-> serde => bytes optional
-> local-tool => ../local-tool

[features]
-> default => [\"json\"]
-> json => [\"dep:serde\"]
";

    #[test]
    fn parse_and_read_fixed_fields() {
        let text = cstr(SAMPLE);
        let handle = hk_parse(text.as_ptr());
        assert_ne!(handle, 0, "expected a valid handle, got error: {}", unsafe {
            from_c(hk_last_error())
        });

        unsafe {
            let name = from_c(hk_get_string(handle, cstr("package.name").as_ptr()));
            assert_eq!(name, "bytes");
            let version = from_c(hk_get_string(handle, cstr("package.version").as_ptr()));
            assert_eq!(version, "0.9");
        }

        hk_free(handle);
    }

    #[test]
    fn enumerate_dynamic_section_keys() {
        let text = cstr(SAMPLE);
        let handle = hk_parse(text.as_ptr());
        assert_ne!(handle, 0);

        let n = hk_map_key_count(handle, cstr("deps").as_ptr());
        assert_eq!(n, 3);

        let keys: Vec<String> = (0..n)
            .map(|i| unsafe { from_c(hk_map_key_at(handle, cstr("deps").as_ptr(), i)) })
            .collect();
        assert_eq!(keys, vec!["hk-parser", "serde", "local-tool"]);

        // Values for each dynamically-discovered key, dotted-path style —
        // exactly how `config::parse`'s rewritten `[deps]` arm reads them.
        unsafe {
            let v0 = from_c(hk_get_string(handle, cstr("deps.hk-parser").as_ptr()));
            assert_eq!(v0, "bytes");
            let v1 = from_c(hk_get_string(handle, cstr("deps.serde").as_ptr()));
            assert_eq!(v1, "bytes optional");
            let v2 = from_c(hk_get_string(handle, cstr("deps.local-tool").as_ptr()));
            assert_eq!(v2, "../local-tool");
        }

        hk_free(handle);
    }

    #[test]
    fn enumerate_root_sections() {
        let text = cstr(SAMPLE);
        let handle = hk_parse(text.as_ptr());
        let n = hk_map_key_count(handle, cstr("").as_ptr());
        assert_eq!(n, 3);
        let sections: Vec<String> = (0..n)
            .map(|i| unsafe { from_c(hk_map_key_at(handle, cstr("").as_ptr(), i)) })
            .collect();
        assert_eq!(sections, vec!["package", "deps", "features"]);
        hk_free(handle);
    }

    #[test]
    fn features_array_values() {
        let text = cstr(SAMPLE);
        let handle = hk_parse(text.as_ptr());
        let len = hk_array_len(handle, cstr("features.json").as_ptr());
        assert_eq!(len, 1);
        unsafe {
            let item = from_c(hk_array_get_string(handle, cstr("features.json").as_ptr(), 0));
            assert_eq!(item, "dep:serde");
        }
        hk_free(handle);
    }

    #[test]
    fn unknown_handle_and_missing_key_are_safe_defaults() {
        assert_eq!(hk_has(999, cstr("package.name").as_ptr()), 0);
        assert_eq!(hk_array_len(999, cstr("deps").as_ptr()), -1);
        assert_eq!(hk_map_key_count(999, cstr("deps").as_ptr()), -1);
        unsafe {
            assert_eq!(from_c(hk_get_string(999, cstr("package.name").as_ptr())), "");
            assert_eq!(from_c(hk_type_of(999, cstr("package.name").as_ptr())), "none");
        }
    }

    #[test]
    fn parse_error_yields_zero_handle_and_sets_last_error() {
        let bad = cstr("not a section header");
        let handle = hk_parse(bad.as_ptr());
        assert_eq!(handle, 0);
        unsafe {
            let err = from_c(hk_last_error());
            assert!(!err.is_empty());
        }
    }
}
