use crate::error::HkError;
use crate::value::{HkConfig, HkValue};
use indexmap::IndexMap;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashSet;
use std::env;

lazy_static! {
    static ref INTERPOL_RE: Regex = Regex::new(r"\$\{([^}]+)\}").unwrap();
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

fn get_value_by_path<'a>(path: &str, config: &'a HkConfig) -> Option<&'a HkValue> {
    let bracket_re = Regex::new(r"([^\[\].]+)(?:\[(\d+)\])?").unwrap();
    let mut parts = Vec::new();
    for cap in bracket_re.captures_iter(path) {
        let key = cap.get(1).map(|m| m.as_str()).unwrap();
        let idx = cap.get(2).map(|m| m.as_str().parse::<usize>().ok());
        parts.push((key, idx.flatten()));
    }

    if parts.is_empty() {
        return None;
    }

    let (first_key, _) = parts[0];
    let mut current_value: Option<&'a HkValue> = config.get(first_key);
    for (key, idx) in parts.iter().skip(1) {
        match current_value {
            Some(HkValue::Map(map)) => {
                current_value = map.get(*key);
            }
            Some(HkValue::Array(arr)) if idx.is_some() => {
                if let Some(i) = idx {
                    if *i < arr.len() {
                        current_value = Some(&arr[*i]);
                        continue;
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        }
        if let Some(idx) = idx {
            if let Some(HkValue::Array(arr)) = current_value {
                if *idx < arr.len() {
                    current_value = Some(&arr[*idx]);
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
    }
    current_value
}
