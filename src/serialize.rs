use crate::value::{HkConfig, HkValue};
use indexmap::IndexMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// Serializes a HkConfig back to a .hk string, preserving key order.
pub fn serialize_hk(config: &HkConfig) -> String {
    let mut output = String::new();
    for (section, value) in config.iter() {
        output.push_str(&format!("[{}]\n", section));
        if let HkValue::Map(map) = value {
            serialize_map(map, 1, &mut output);
        }
        output.push('\n');
    }
    output.trim_end().to_string()
}

fn serialize_map(map: &IndexMap<String, HkValue>, level: usize, output: &mut String) {
    let prefix = "-".repeat(level) + " > ";
    for (key, value) in map.iter() {
        match value {
            HkValue::Map(submap) => {
                output.push_str(&format!("{}{}\n", prefix, key));
                serialize_map(submap, level + 1, output);
            }
            _ => {
                let val = serialize_value(value);
                output.push_str(&format!("{}{} => {}\n", prefix, key, val));
            }
        }
    }
}

fn serialize_value(value: &HkValue) -> String {
    match value {
        HkValue::String(s) => {
            if s.is_empty() || s.contains(',') || s.contains(' ') || s.contains(']') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace("\"", "\\\""))
            } else {
                s.clone()
            }
        }
        HkValue::Number(n) => n.to_string(),
        HkValue::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        HkValue::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(serialize_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        HkValue::Map(_) => "<map>".to_string(),
    }
}

pub fn write_hk_file<P: AsRef<Path>>(path: P, config: &HkConfig) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(serialize_hk(config).as_bytes())
}
