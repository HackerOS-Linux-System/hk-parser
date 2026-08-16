use crate::error::HkError;
use crate::value::{HkConfig, HkValue};
use indexmap::IndexMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

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

            // Find the end of this section (next section or EOF).
            //
            // FIXED (v3.2.1): this used to be a plain `next_line.starts_with('[')`
            // check, which misfired on a multi-line array item that's itself an
            // array, e.g.:
            //   -> groups => [
            //       ["admins", "root"]   <- trim_start() starts with '[' too!
            //       ["users", "guest"]
            //   ]
            // ...prematurely ending the section right after `-> groups => [`, so
            // `parse_map` below only ever saw that one line and had no closing
            // `]` left to find ("Unclosed array"). We now track array-bracket
            // depth (outside quotes) across the scan and only treat a `[`-led
            // line as a new section header while that depth is back to zero —
            // i.e. we're not currently inside someone's still-open array value.
            let mut end = i + 1;
            let mut array_depth: i32 = 0;
            while end < lines.len() {
                let next_line = lines[end];
                let next_trimmed = next_line.trim_start();
                if array_depth == 0 && next_trimmed.starts_with('[') {
                    break;
                }
                array_depth += net_bracket_depth(next_line);
                end += 1;
            }

            let section_lines = &lines[i + 1..end];
            // `section_lines[0]` is `lines[i + 1]` (0-indexed), whose true
            // 1-indexed line number is `(i + 1) + 1 = i + 2`.
            let map = parse_map(1, section_lines, i + 2)?;
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
        if dash_count < level {
            // Shallower level – legitimately return control to the caller.
            break;
        }
        if dash_count > level {
            // The line is MORE indented than expected at this depth. This means
            // one or more nesting levels were skipped (e.g. jumping from "-->"
            // straight to "---->" without a "--->" level in between). Previously
            // this was silently treated the same as "end of this map", which made
            // the parser drop the entire mismatched sub-tree without any warning.
            // That is a data-loss bug, so we now report it as a proper parse error.
            return Err(HkError::Parse {
                line: (start_line + i) as u32,
                column: 1,
                message: format!(
                    "Inconsistent nesting level: expected {} dash(es) (\"{}\") at this depth, found {} (\"{}\"). Nesting must increase by exactly one dash per level.",
                    level,
                    "-".repeat(level),
                    dash_count,
                    "-".repeat(dash_count)
                ),
            });
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
            let value_col = arrow_pos + dash_count + 2;

            // Multi-line arrays.
            //
            // `[1, 2, 3]` on a single line already worked. This adds the
            // second common style:
            //
            //   -> tags => [
            //       "desktop"
            //       "environment"
            //   ]
            //
            // one item per line (trailing commas optional), closed by a
            // line containing the matching `]`. Detected by: the value
            // starts with `[` but doesn't already balance back to zero
            // brackets on this same line.
            if value_part.starts_with('[') && net_bracket_depth(value_part) > 0 {
                let mut buf = value_part.to_string();
                let mut consumed = 1usize;
                let mut j = i + 1;
                while net_bracket_depth(&buf) > 0 {
                    if j >= lines.len() {
                        return Err(HkError::Parse {
                            line: (start_line + i) as u32,
                            column: value_col,
                            message: "Unclosed array: reached end of section before a matching ']'".to_string(),
                        });
                    }
                    buf.push('\n');
                    buf.push_str(lines[j]);
                    consumed += 1;
                    j += 1;
                }
                // `buf` is now e.g. "[\n    \"desktop\"\n    \"environment\"\n]".
                let first = buf.find('[').unwrap();
                let last = buf.rfind(']').unwrap();
                let inner = &buf[first + 1..last];
                let items = parse_array_inner(inner, start_line + i, value_col)?;
                insert_key(&mut map, &key, HkValue::Array(items))?;
                i += consumed;
            } else {
                let value = parse_value(value_part, start_line + i, value_col)?;
                insert_key(&mut map, &key, value)?;
                i += 1;
            }
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

    // Array (single-line form: `[1, 2, 3]`; the multi-line form is handled
    // one level up, in `parse_map`, before `parse_value` is ever called on
    // it — by the time we get here for an array it's always one complete,
    // already-balanced `[...]` string, single- or multi-line alike).
    if s.starts_with('[') && s.ends_with(']') && net_bracket_depth(s) == 0 {
        let inner = &s[1..s.len() - 1];
        let items = parse_array_inner(inner, line, column)?;
        Ok(HkValue::Array(items))
    } else {
        parse_simple_value(s, line, column)
    }
}

/// Net count of `[` minus `]` seen outside of quoted strings. Zero means
/// the brackets in `s` are balanced (a complete, self-contained value);
/// positive means an array was opened but not yet closed within `s`
/// (used to detect that an array continues on the following lines).
fn net_bracket_depth(s: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_quotes = false;
    let mut escape = false;
    for c in s.chars() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escape = true,
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes => depth += 1,
            ']' if !in_quotes => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Parses the inside of an array (the text strictly between the outer `[`
/// and its matching `]`) into items. Supports both separator styles so
/// callers don't need to know which one was used:
///   - commas, for the classic single-line style: `1, 2, "three"`
///   - newlines, for the multi-line style (one item per line; a trailing
///     comma on each line is fine but not required)
/// Nested arrays (`[1, [2, 3], 4]`, on one line or spread across several)
/// are parsed recursively — commas/newlines inside a nested `[...]` don't
/// split the outer array. Quotes are preserved into each item's own text
/// so `parse_value`/`parse_simple_value` can still tell a quoted string
/// apart from a bare number/bool/word.
fn parse_array_inner(inner: &str, line: usize, column: usize) -> Result<Vec<HkValue>, HkError> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escape = false;
    let mut depth = 0i32;

    macro_rules! flush_item {
        () => {{
            let trimmed = current.trim();
            let trimmed = trimmed.strip_suffix(',').unwrap_or(trimmed).trim();
            if !trimmed.is_empty() && !trimmed.starts_with('!') {
                items.push(parse_value(trimmed, line, column)?);
            }
            current.clear();
        }};
    }

    for c in inner.chars() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }
        match c {
            '\\' if in_quotes => {
                current.push(c);
                escape = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            '[' if !in_quotes => {
                depth += 1;
                current.push(c);
            }
            ']' if !in_quotes => {
                depth -= 1;
                current.push(c);
            }
            ',' if !in_quotes && depth == 0 => flush_item!(),
            '\n' if !in_quotes && depth == 0 => flush_item!(),
            _ => current.push(c),
        }
    }
    flush_item!();
    Ok(items)
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
