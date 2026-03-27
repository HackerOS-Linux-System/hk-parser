use indexmap::IndexMap;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;
use colored::Colorize;

/// Represents the structure of a .hk file.
/// Sections are top-level keys in the outer IndexMap to preserve order.
pub type HkConfig = IndexMap<String, HkValue>;

lazy_static! {
    static ref INTERPOL_RE: Regex = Regex::new(r"\$\{([^}]+)\}").unwrap();
}

/// Enum for values in the .hk config: supports strings, numbers, booleans, arrays, and maps.
#[derive(Debug, Clone, PartialEq)]
pub enum HkValue {
    String(String),
    Number(f64),
    Bool(bool),
    Array(Vec<HkValue>),
    Map(IndexMap<String, HkValue>),
}

impl HkValue {
    pub fn as_string(&self) -> Result<String, HkError> {
        match self {
            Self::String(s) => Ok(s.clone()),
            Self::Number(n) => Ok(n.to_string()),
            Self::Bool(b) => Ok(b.to_string()),
            _ => Err(HkError::TypeMismatch {
                expected: "string".to_string(),
                found: format!("{:?}", self),
            }),
        }
    }

    pub fn as_number(&self) -> Result<f64, HkError> {
        if let Self::Number(n) = self {
            Ok(*n)
        } else {
            Err(HkError::TypeMismatch {
                expected: "number".to_string(),
                found: format!("{:?}", self),
            })
        }
    }

    pub fn as_bool(&self) -> Result<bool, HkError> {
        if let Self::Bool(b) = self {
            Ok(*b)
        } else {
            Err(HkError::TypeMismatch {
                expected: "bool".to_string(),
                found: format!("{:?}", self),
            })
        }
    }

    pub fn as_array(&self) -> Result<&Vec<HkValue>, HkError> {
        if let Self::Array(a) = self {
            Ok(a)
        } else {
            Err(HkError::TypeMismatch {
                expected: "array".to_string(),
                found: format!("{:?}", self),
            })
        }
    }

    pub fn as_map(&self) -> Result<&IndexMap<String, HkValue>, HkError> {
        if let Self::Map(m) = self {
            Ok(m)
        } else {
            Err(HkError::TypeMismatch {
                expected: "map".to_string(),
                found: format!("{:?}", self),
            })
        }
    }
}

/// Custom error type for parsing .hk files.
#[derive(Error, Debug)]
pub enum HkError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Parse error at line {line}, column {column}: {message}")]
    Parse {
        line: u32,
        column: usize,
        message: String,
    },
    #[error("Type mismatch: expected {expected}, found {found}")]
    TypeMismatch { expected: String, found: String },
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Invalid reference: {0}")]
    InvalidReference(String),
    #[error("Cyclic reference detected: {0}")]
    CyclicReference(String),
    #[error("Key conflict: {0}")]
    KeyConflict(String),
}

impl HkError {
    pub fn pretty_print(&self, source: &str) {
        match self {
            Self::Parse { line, column, message } => {
                eprintln!("{} {}", "error:".red().bold(), "parse error".red().bold());
                eprintln!("  {} at {}:{}", "→".red(), line, column);
                if let Some(line_content) = source.lines().nth((*line - 1) as usize) {
                    eprintln!("\n  {}", line_content);
                    eprintln!("  {}{}", " ".repeat(*column), "^".red().bold());
                    eprintln!("  {}", message.red());
                } else {
                    eprintln!("  {}", message.red());
                }

                if message.contains("tag \"=>\"") {
                    eprintln!("\n{} {}", "Hint:".yellow().bold(), "Try: key => value".cyan());
                } else if message.contains("tag \"[\"") {
                    eprintln!("\n{} {}", "Hint:".yellow().bold(), "Sections must start with [name]".cyan());
                } else if message.contains("take_while1") {
                    eprintln!("\n{} {}", "Hint:".yellow().bold(), "Keys can only contain letters, digits, '_', '-', '.'".cyan());
                }
            }
            Self::TypeMismatch { expected, found } => {
                eprintln!("{} {}", "error:".red().bold(), "type mismatch".red().bold());
                eprintln!("  expected {}, got {}", expected.cyan(), found.red());
            }
            Self::InvalidReference(ref_var) => {
                eprintln!("{} {}", "error:".red().bold(), "invalid reference".red().bold());
                eprintln!("  {}", ref_var.red());
                eprintln!("\n{} {}", "Hint:".yellow().bold(), "Check if the referenced key exists and is accessible".cyan());
            }
            Self::CyclicReference(path) => {
                eprintln!("{} {}", "error:".red().bold(), "cyclic reference".red().bold());
                eprintln!("  {}", path.red());
            }
            Self::KeyConflict(key) => {
                eprintln!("{} {}", "error:".red().bold(), "key conflict".red().bold());
                eprintln!("  Duplicate key '{}' in nested structure", key.red());
            }
            _ => eprintln!("{}", self.to_string().red()),
        }
    }
}

/// Parses a .hk file from a string input.
pub fn parse_hk(input: &str) -> Result<HkConfig, HkError> {
    let lines: Vec<&str> = input.lines().collect();
    let mut config = IndexMap::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_start();
        if line.is_empty() || line.starts_with('!') {
            i += 1;
            continue;
        }

        if line.starts_with('[') {
            let close = line.find(']').ok_or_else(|| HkError::Parse {
                line: (i + 1) as u32,
                column: line.find('[').unwrap() + 1,
                message: "Unclosed section header".to_string(),
            })?;
            let section_name = line[1..close].trim();
            if section_name.is_empty() {
                return Err(HkError::Parse {
                    line: (i + 1) as u32,
                    column: close + 1,
                    message: "Empty section name".to_string(),
                });
            }

            // Find the end of this section (next section or EOF)
            let mut end = i + 1;
            while end < lines.len() {
                let next_line = lines[end].trim_start();
                if next_line.starts_with('[') {
                    break;
                }
                end += 1;
            }

            let section_lines = &lines[i + 1..end];
            let map = parse_map(1, section_lines, i + 1)?;
            config.insert(section_name.to_string(), HkValue::Map(map));
            i = end;
        } else {
            return Err(HkError::Parse {
                line: (i + 1) as u32,
                column: 1,
                message: "Expected section header".to_string(),
            });
        }
    }

    Ok(config)
}

/// Parse a map from a slice of lines, starting with a given indentation level (number of dashes).
/// level: the number of dashes expected for the current depth (e.g., 1 for "->", 2 for "-->")
/// Returns the map and the index of the next line to process.
fn parse_map(level: usize, lines: &[&str], start_line: usize) -> Result<IndexMap<String, HkValue>, HkError> {
    let mut map = IndexMap::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('!') {
            i += 1;
            continue;
        }

        // Count leading dashes
        let dash_count = trimmed.chars().take_while(|c| *c == '-').count();
        if dash_count == 0 {
            return Err(HkError::Parse {
                line: (start_line + i) as u32,
                column: 1,
                message: "Expected key or map header".to_string(),
            });
        }
        if dash_count != level {
            // Different level – return to caller
            break;
        }

        // After dashes, skip any spaces, expect '>', then skip spaces
        let after_dashes = &trimmed[dash_count..];
        let rest = after_dashes.trim_start();
        if !rest.starts_with('>') {
            return Err(HkError::Parse {
                line: (start_line + i) as u32,
                column: dash_count + 1,
                message: "Expected '>' after dashes".to_string(),
            });
        }
        let after_gt = &rest[1..].trim_start();
        if after_gt.is_empty() {
            return Err(HkError::Parse {
                line: (start_line + i) as u32,
                column: dash_count + 1,
                message: "Missing key after '>'".to_string(),
            });
        }

        // Check if it's a key-value line (contains "=>")
        if let Some(arrow_pos) = after_gt.find("=>") {
            let key = after_gt[..arrow_pos].trim();
            let value_part = after_gt[arrow_pos + 2..].trim();
            let key = unquote_key(key);
            if key.is_empty() {
                return Err(HkError::Parse {
                    line: (start_line + i) as u32,
                    column: dash_count + 1,
                    message: "Empty key".to_string(),
                });
            }
            let value = parse_value(value_part, start_line + i, arrow_pos + dash_count + 2)?;
            insert_key(&mut map, &key, value)?;
            i += 1;
        } else {
            // It's a map header: "- > key" without "=>"
            let key = after_gt.trim();
            let key = unquote_key(key);
            if key.is_empty() {
                return Err(HkError::Parse {
                    line: (start_line + i) as u32,
                    column: dash_count + 1,
                    message: "Empty map key".to_string(),
                });
            }

            // Find the sub-lines that belong to this map (higher level)
            let next_level = level + 1;
            let mut j = i + 1;
            while j < lines.len() {
                let sub_line = lines[j];
                let sub_trimmed = sub_line.trim_start();
                if sub_trimmed.is_empty() || sub_trimmed.starts_with('!') {
                    j += 1;
                    continue;
                }
                let sub_dash_count = sub_trimmed.chars().take_while(|c| *c == '-').count();
                if sub_dash_count < next_level {
                    break;
                }
                j += 1;
            }

            let sub_lines = &lines[i + 1..j];
            let sub_map = parse_map(next_level, sub_lines, start_line + i + 1)?;
            insert_key(&mut map, &key, HkValue::Map(sub_map))?;
            i = j;
        }
    }

    Ok(map)
}

/// Insert a key (which may contain dots for nesting) into the map.
/// Keys that start or end with a dot are treated as literal keys (no nesting).
fn insert_key(map: &mut IndexMap<String, HkValue>, key: &str, value: HkValue) -> Result<(), HkError> {
    // If the key contains dots but not at the start or end, split and nest.
    if key.contains('.') && !key.starts_with('.') && !key.ends_with('.') {
        let parts: Vec<&str> = key.split('.').collect();
        insert_nested(map, parts, value)
    } else {
        // Otherwise, treat as a single key.
        if map.contains_key(key) {
            return Err(HkError::KeyConflict(key.to_string()));
        }
        map.insert(key.to_string(), value);
        Ok(())
    }
}

/// Insert a nested key using the split parts.
fn insert_nested(map: &mut IndexMap<String, HkValue>, keys: Vec<&str>, value: HkValue) -> Result<(), HkError> {
    let mut current = map;
    for key in &keys[0..keys.len() - 1] {
        let entry = current
            .entry(key.to_string())
            .or_insert(HkValue::Map(IndexMap::new()));
        if let HkValue::Map(submap) = entry {
            current = submap;
        } else {
            return Err(HkError::KeyConflict(key.to_string()));
        }
    }
    if let Some(last_key) = keys.last() {
        current.insert(last_key.to_string(), value);
    }
    Ok(())
}

/// Remove surrounding quotes from a key (if present) and unescape inner quotes.
fn unquote_key(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        inner.replace("\\\"", "\"")
    } else {
        s.to_string()
    }
}

fn parse_value(s: &str, line: usize, column: usize) -> Result<HkValue, HkError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(HkError::Parse {
            line: line as u32,
            column,
            message: "Empty value".to_string(),
        });
    }

    // Array
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        let mut items = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut escape = false;
        for c in inner.chars() {
            if escape {
                current.push(c);
                escape = false;
                continue;
            }
            match c {
                '\\' => escape = true,
                '"' => in_quotes = !in_quotes,
                ',' if !in_quotes => {
                    if !current.trim().is_empty() {
                        let item = parse_simple_value(current.trim(), line, column)?;
                        items.push(item);
                        current.clear();
                    }
                }
                _ => current.push(c),
            }
        }
        if !current.trim().is_empty() {
            let item = parse_simple_value(current.trim(), line, column)?;
            items.push(item);
        }
        Ok(HkValue::Array(items))
    } else {
        parse_simple_value(s, line, column)
    }
}

fn parse_simple_value(s: &str, line: usize, column: usize) -> Result<HkValue, HkError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(HkError::Parse {
            line: line as u32,
            column,
            message: "Empty value".to_string(),
        });
    }

    // Boolean
    if s.eq_ignore_ascii_case("true") {
        return Ok(HkValue::Bool(true));
    }
    if s.eq_ignore_ascii_case("false") {
        return Ok(HkValue::Bool(false));
    }

    // Number
    if let Ok(n) = f64::from_str(s) {
        return Ok(HkValue::Number(n));
    }

    // Quoted string
    if s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut result = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    match next {
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        _ => result.push(next),
                    }
                }
            } else {
                result.push(c);
            }
        }
        Ok(HkValue::String(result))
    } else {
        // Plain string
        Ok(HkValue::String(s.to_string()))
    }
}

/// Loads and parses a .hk file from the given path.
pub fn load_hk_file<P: AsRef<Path>>(path: P) -> Result<HkConfig, HkError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut contents = String::new();
    for line in reader.lines() {
        let line = line?;
        contents.push_str(&line);
        contents.push('\n');
    }
    parse_hk(&contents)
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
            if s.contains(',') || s.contains(' ') || s.contains(']') || s.contains('"') || s.contains('\n') {
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_parse_libraries_repo() {
        let input = r#"
! Repozytorium bibliotek dla Hacker Lang

[libraries]
-> obsidian
--> version => 0.2
--> description => Biblioteka inspirowana zenity.
--> authors => ["HackerOS Team <hackeros068@gmail.com>"]
--> so-download => https://github.com/Bytes-Repository/obsidian-lib/releases/download/v0.2/libobsidian_lib.so
--> .hl-download => https://github.com/Bytes-Repository/obsidian-lib/blob/main/obsidian.hl

-> yuy
--> version => 0.2
--> description => Twórz ładne interfejsy cli
"#;
        let result = parse_hk(input).expect("Failed to parse libraries file");
        assert!(result.contains_key("libraries"));
        let libraries = result["libraries"].as_map().unwrap();
        assert!(libraries.contains_key("obsidian"));
        let obsidian = libraries["obsidian"].as_map().unwrap();
        assert_eq!(obsidian["version"].as_number().unwrap(), 0.2);
        assert_eq!(obsidian["description"].as_string().unwrap(), "Biblioteka inspirowana zenity.");
        assert!(obsidian.contains_key("so-download"));
        assert!(obsidian.contains_key(".hl-download"));
        assert_eq!(
            obsidian[".hl-download"].as_string().unwrap(),
            "https://github.com/Bytes-Repository/obsidian-lib/blob/main/obsidian.hl"
        );

        assert!(libraries.contains_key("yuy"));
        let yuy = libraries["yuy"].as_map().unwrap();
        assert_eq!(yuy["version"].as_number().unwrap(), 0.2);
    }

    #[test]
    fn test_parse_hk_with_comments_and_types() {
        let input = r#"
        ! Globalne informacje o projekcie
        [metadata]
        -> name => Hacker Lang
        -> version => 1.5
        -> list => [1, 2.5, true, "four"]
        "#;
        let result = parse_hk(input).unwrap();
        assert!(result.contains_key("metadata"));
        let metadata = result["metadata"].as_map().unwrap();
        assert_eq!(metadata["name"].as_string().unwrap(), "Hacker Lang");
        assert_eq!(metadata["version"].as_number().unwrap(), 1.5);
        let list = metadata["list"].as_array().unwrap();
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn test_edge_cases() {
        // Empty section
        let input = "[empty]\n";
        let config = parse_hk(input).unwrap();
        assert!(config.contains_key("empty"));
        assert_eq!(config["empty"].as_map().unwrap().len(), 0);

        // Section with only comments
        let input = "[comments]\n! comment\n! another\n";
        let config = parse_hk(input).unwrap();
        assert!(config.contains_key("comments"));
        assert_eq!(config["comments"].as_map().unwrap().len(), 0);

        // Nested map with dots in keys
        let input = r#"
[config]
-> a.b.c => 42
"#;
        let config = parse_hk(input).unwrap();
        let a = config["config"].as_map().unwrap().get("a").unwrap().as_map().unwrap();
        let b = a.get("b").unwrap().as_map().unwrap();
        let c = b.get("c").unwrap().as_number().unwrap();
        assert_eq!(c, 42.0);
    }

    #[test]
    fn test_array_reference() {
        let input = r#"
[data]
-> numbers => [10, 20, 30]
-> first => ${data.numbers[0]}
"#;
        let mut config = parse_hk(input).unwrap();
        resolve_interpolations(&mut config).unwrap();
        let first = config["data"].as_map().unwrap()["first"].as_string().unwrap();
        assert_eq!(first, "10");
    }

    #[test]
    fn test_cyclic_reference_detection() {
        let input = r#"
[a]
-> b => ${a.c}
-> c => ${a.b}
"#;
        let mut config = parse_hk(input).unwrap();
        let err = resolve_interpolations(&mut config).unwrap_err();
        match err {
            HkError::CyclicReference(path) => {
                assert!(path.contains("a.b") || path.contains("a.c"));
            }
            _ => panic!("Expected cyclic reference error, got {:?}", err),
        }
    }

    #[test]
    fn test_key_conflict() {
        let input = r#"
[conflict]
-> a => 1
-> a.b => 2
"#;
        let result = parse_hk(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_reference() {
        let input = r#"
[a]
-> b => ${a.missing}
"#;
        let mut config = parse_hk(input).unwrap();
        let err = resolve_interpolations(&mut config).unwrap_err();
        match err {
            HkError::InvalidReference(var) => {
                assert_eq!(var, "a.missing");
            }
            _ => panic!("Expected invalid reference error"),
        }
    }

    #[test]
    fn test_serialize_roundtrip() {
        let input = r#"
[test]
-> key => value
-> array => [1, "two", true]
-> nested
--> sub => 42
"#;
        let config = parse_hk(input).unwrap();
        let serialized = serialize_hk(&config);
        let parsed_again = parse_hk(&serialized).unwrap();
        assert_eq!(config, parsed_again);
    }
}
