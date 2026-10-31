use crate::error::HkError;
use crate::value::{HkConfig, HkValue};
use indexmap::IndexMap;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashSet;
use std::env;

lazy_static! {
    static ref INTERPOL_RE: Regex = Regex::new(r"\$\{([^}]+)\}").unwrap();
    /// One `key` or `key[idx]` step of a dotted path. Compiled once (it used
    /// to be rebuilt on every `get_value_by_path` call).
    /// Read-side step: `key` plus any number of `[n]` suffixes.
    static ref GET_RE: Regex = Regex::new(r"([^\[\].]+)((?:\[\d+\])*)").unwrap();
    static ref INDEX_RE: Regex = Regex::new(r"\[(\d+)\]").unwrap();
    static ref PATH_RE: Regex = Regex::new(r"([^\[\].]+)(?:\[(\d+)\])?").unwrap();
}

/// Resolves interpolations in the config, including env vars and references.
pub fn resolve_interpolations(config: &mut HkConfig) -> Result<(), HkError> {
    let context = config.clone();
    let mut resolved = HashSet::new();
    let mut resolving = Vec::new();
    for (section, value) in config.iter_mut() {
        if let HkValue::Map(map) = value {
            resolve_map(map, &context, &mut resolved, &mut resolving, &format!("{}", section))?;
        }
    }
    Ok(())
}

fn resolve_map(
    map: &mut IndexMap<String, HkValue>,
    top: &HkConfig,
    resolved: &mut HashSet<String>,
    resolving: &mut Vec<String>,
    path: &str,
) -> Result<(), HkError> {
    for (key, v) in map.iter_mut() {
        let new_path = format!("{}.{}", path, key);
        if resolved.contains(&new_path) {
            continue;
        }
        resolving.push(new_path.clone());
        resolve_value(v, top, resolved, resolving, &new_path)?;
        resolving.pop();
        resolved.insert(new_path);
    }
    Ok(())
}

fn resolve_value(
    v: &mut HkValue,
    top: &HkConfig,
    resolved: &mut HashSet<String>,
    resolving: &mut Vec<String>,
    path: &str,
) -> Result<(), HkError> {
    match v {
        HkValue::String(s) => {
            let mut new_s = String::new();
            let mut last = 0;
            for cap in INTERPOL_RE.captures_iter(s) {
                let m = cap.get(0).unwrap();
                new_s.push_str(&s[last..m.start()]);
                let var = &cap[1];
                let repl = if var.starts_with("env:") {
                    env::var(&var[4..]).unwrap_or_default()
                } else {
                    // Resolve the reference recursively, detecting cycles
                    if resolving.contains(&var.to_string()) {
                        return Err(HkError::CyclicReference(var.to_string()));
                    }
                    resolve_reference(var, top, resolved, resolving)?
                };
                new_s.push_str(&repl);
                last = m.end();
            }
            new_s.push_str(&s[last..]);
            *s = new_s;
        }
        HkValue::Array(a) => {
            for (i, item) in a.iter_mut().enumerate() {
                resolve_value(item, top, resolved, resolving, &format!("{}[{}]", path, i))?;
            }
        }
        HkValue::Map(m) => {
            resolve_map(m, top, resolved, resolving, path)?;
        }
        _ => {}
    }
    Ok(())
}

fn resolve_reference(
    path: &str,
    top: &HkConfig,
    resolved: &mut HashSet<String>,
    resolving: &mut Vec<String>,
) -> Result<String, HkError> {
    // Check if the reference is already in the resolving stack (cycle)
    if resolving.contains(&path.to_string()) {
        return Err(HkError::CyclicReference(path.to_string()));
    }

    // Get the raw value from the config
    let raw_value = get_value_by_path(path, top).ok_or_else(|| HkError::InvalidReference(path.to_string()))?;
    // Clone the value so we can resolve it without affecting the original
    let mut cloned_value = raw_value.clone();

    // Push the path onto the resolving stack
    resolving.push(path.to_string());

    // Resolve the cloned value recursively
    resolve_value(&mut cloned_value, top, resolved, resolving, path)?;

    // Pop the path from the stack
    resolving.pop();

    // Convert the resolved value to a string
    cloned_value.as_string()
}

/// Public (was `pub(crate)`, and private before that) so Rust callers, the
/// Lua bindings and `ffi.rs`'s dotted-path lookups
/// (`hk_has`/`hk_get_*`/`hk_type_of`/`hk_map_key_*` — see that module's
/// doc comment) can reuse the exact same `"section.key[idx]"` resolution
/// `${...}` interpolation already relies on, instead of a second,
/// independently-maintained copy of the same walk that could drift out
/// of sync with this one.
pub fn get_value_by_path<'a>(path: &str, config: &'a HkConfig) -> Option<&'a HkValue> {
    // Each step is `key` followed by zero or more `[n]`, so `a.b[4][0]`
    // (an element of a nested array) resolves too.
    let mut steps = GET_RE.captures_iter(path).map(|cap| {
        let key = cap.get(1).unwrap().as_str();
        let indices: Vec<usize> = INDEX_RE
            .captures_iter(cap.get(2).map_or("", |m| m.as_str()))
            .filter_map(|c| c[1].parse::<usize>().ok())
            .collect();
        (key, indices)
    });

    let (first_key, first_idx) = steps.next()?;
    let mut cur: &'a HkValue = config.get(first_key)?;
    cur = index_into(cur, &first_idx)?;
    for (key, idx) in steps {
        cur = match cur {
            HkValue::Map(m) => m.get(key)?,
            _ => return None,
        };
        cur = index_into(cur, &idx)?;
    }
    Some(cur)
}

fn index_into<'a>(mut v: &'a HkValue, indices: &[usize]) -> Option<&'a HkValue> {
    for i in indices {
        v = match v {
            HkValue::Array(a) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(v)
}

// ─── Path-based mutation ────────────────────────────────────────────────────
//
// `get_value_by_path` above is read-only. The functions below add the write
// side with the *same* path syntax (`"section.key"`, `"section.list[2]"`),
// plus a segment-based variant for keys that themselves contain a `.` or a
// bracket (e.g. the `.hl-download` key used by the libraries repository
// format, which no dotted string path can address).

/// One step of a path: a map key, optionally followed by ONE array index.
/// (Reads via `get_value_by_path` also accept chained `a[1][0]`; the mutation
/// functions below take a single index per step.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSeg {
    pub key: String,
    pub index: Option<usize>,
}

impl PathSeg {
    /// A plain map-key step (no index).
    pub fn key(key: impl Into<String>) -> Self {
        Self { key: key.into(), index: None }
    }
}

/// Splits `"a.b[2].c"` into its steps, with the exact tokenisation
/// `get_value_by_path` uses.
pub fn parse_path(path: &str) -> Vec<PathSeg> {
    PATH_RE
        .captures_iter(path)
        .map(|cap| PathSeg {
            key: cap[1].to_string(),
            index: cap.get(2).and_then(|m| m.as_str().parse::<usize>().ok()),
        })
        .collect()
}

/// Map-only lookup by already-split keys: no `.`/`[]` interpretation at all,
/// so any key name works.
pub fn get_value_by_keys<'a>(config: &'a HkConfig, keys: &[&str]) -> Option<&'a HkValue> {
    let (first, rest) = keys.split_first()?;
    let mut cur = config.get(*first)?;
    for k in rest {
        cur = match cur {
            HkValue::Map(m) => m.get(*k)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn describe(segs: &[PathSeg]) -> String {
    segs.iter()
        .map(|s| match s.index {
            Some(i) => format!("{}[{}]", s.key, i),
            None => s.key.clone(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Sets the value at a dotted path, creating missing intermediate *maps*
/// (but never arrays). An existing key keeps its position, a new key is
/// appended — so `serialize_hk` output stays stable.
///
/// Rules:
/// * the first step is a section and must end up a `Map` (a bare scalar at
///   the top level isn't representable in `.hk`);
/// * `list[i]` replaces element `i`, or appends when `i == len`;
/// * passing through an existing non-map value is a `TypeMismatch`.
pub fn set_value_by_path(config: &mut HkConfig, path: &str, value: HkValue) -> Result<(), HkError> {
    set_value_by_segments(config, &parse_path(path), value)
}

/// Same as [`set_value_by_path`] for pre-split steps.
pub fn set_value_by_segments(config: &mut HkConfig, segs: &[PathSeg], value: HkValue) -> Result<(), HkError> {
    if segs.is_empty() {
        return Err(HkError::InvalidReference("<empty path>".to_string()));
    }
    if segs[0].index.is_some() {
        return Err(HkError::InvalidReference(format!(
            "{} (a top-level section cannot be indexed)",
            describe(segs)
        )));
    }
    if segs.len() == 1 && !matches!(value, HkValue::Map(_)) {
        return Err(HkError::TypeMismatch {
            expected: "map (top-level entries are sections)".to_string(),
            found: value.type_name().to_string(),
        });
    }
    set_in_map(config, segs, value, segs)
}

fn set_in_map(
    map: &mut IndexMap<String, HkValue>,
    segs: &[PathSeg],
    value: HkValue,
    full: &[PathSeg],
) -> Result<(), HkError> {
    let seg = &segs[0];
    let last = segs.len() == 1;

    match (&seg.index, last) {
        // `key => value`
        (None, true) => {
            map.insert(seg.key.clone(), value);
            Ok(())
        }
        // `key[i] => value`
        (Some(i), true) => {
            let arr = match map.get_mut(&seg.key) {
                Some(HkValue::Array(a)) => a,
                Some(other) => {
                    return Err(HkError::TypeMismatch {
                        expected: "array".to_string(),
                        found: other.type_name().to_string(),
                    })
                }
                None => return Err(HkError::MissingField(describe(full))),
            };
            if *i < arr.len() {
                arr[*i] = value;
            } else if *i == arr.len() {
                arr.push(value);
            } else {
                return Err(HkError::MissingField(describe(full)));
            }
            Ok(())
        }
        // descend into `key` (or `key[i]`)
        (idx, false) => {
            let child: &mut HkValue = match idx {
                None => map
                    .entry(seg.key.clone())
                    .or_insert_with(|| HkValue::Map(IndexMap::new())),
                Some(i) => match map.get_mut(&seg.key) {
                    Some(HkValue::Array(a)) => {
                        a.get_mut(*i).ok_or_else(|| HkError::MissingField(describe(full)))?
                    }
                    Some(other) => {
                        return Err(HkError::TypeMismatch {
                            expected: "array".to_string(),
                            found: other.type_name().to_string(),
                        })
                    }
                    None => return Err(HkError::MissingField(describe(full))),
                },
            };
            match child {
                HkValue::Map(m) => set_in_map(m, &segs[1..], value, full),
                other => Err(HkError::TypeMismatch {
                    expected: "map".to_string(),
                    found: other.type_name().to_string(),
                }),
            }
        }
    }
}

/// Removes and returns the value at a dotted path (`None` if absent).
/// Removing a map key keeps the order of the remaining keys; removing
/// `list[i]` shifts later elements down.
pub fn remove_value_by_path(config: &mut HkConfig, path: &str) -> Option<HkValue> {
    remove_value_by_segments(config, &parse_path(path))
}

/// Same as [`remove_value_by_path`] for pre-split steps.
pub fn remove_value_by_segments(config: &mut HkConfig, segs: &[PathSeg]) -> Option<HkValue> {
    if segs.is_empty() || segs[0].index.is_some() {
        return None;
    }
    remove_in_map(config, segs)
}

fn remove_in_map(map: &mut IndexMap<String, HkValue>, segs: &[PathSeg]) -> Option<HkValue> {
    let seg = &segs[0];
    if segs.len() == 1 {
        return match seg.index {
            None => map.shift_remove(&seg.key),
            Some(i) => match map.get_mut(&seg.key) {
                Some(HkValue::Array(a)) if i < a.len() => Some(a.remove(i)),
                _ => None,
            },
        };
    }
    let child = match seg.index {
        None => map.get_mut(&seg.key)?,
        Some(i) => match map.get_mut(&seg.key)? {
            HkValue::Array(a) => a.get_mut(i)?,
            _ => return None,
        },
    };
    match child {
        HkValue::Map(m) => remove_in_map(m, &segs[1..]),
        _ => None,
    }
}
