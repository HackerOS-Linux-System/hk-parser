// src/lib.rs
//! Hacker Lang Configuration Parser
//!
//! This crate provides a robust parser and serializer for .hk files used in Hacker Lang,
//! the programming language for HackerOS. It supports nested structures, comments, and
//! error handling.
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::{multispace0, multispace1, newline, not_line_ending, satisfy},
    combinator::{map, opt, recognize},
    error::context,
    multi::{many0, many1},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use thiserror::Error;
/// Represents the structure of a .hk file.
/// Sections are top-level keys in the outer HashMap.
/// Values can be simple strings or nested HashMaps for subsections.
pub type HkConfig = HashMap<String, HkValue>;
/// Enum for values in the .hk config: either a simple string or a nested map.
#[derive(Debug, Clone, PartialEq)]
pub enum HkValue {
    String(String),
    Map(HashMap<String, HkValue>),
}
/// Custom error type for parsing .hk files.
#[derive(Error, Debug)]
pub enum HkError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Parse error at line {line}: {message}")]
    Parse { line: usize, message: String },
}
/// Parses a .hk file from a string input.
pub fn parse_hk(input: &str) -> Result<HkConfig, HkError> {
    let mut line_num = 1;
    let mut remaining = input;
    let mut config = HashMap::new();
    while !remaining.is_empty() {
        let (rest, _) = match multispace0::<&str, nom::error::Error<&str>>(remaining) {
            Ok(v) => v,
            Err(e) => return Err(HkError::Parse {
                line: line_num,
                message: e.to_string(),
            }),
        };
        remaining = rest;
        if remaining.is_empty() {
            break;
        }
        if remaining.starts_with('!') {
            // Skip comment line
            let (rest, _) = match comment(remaining) {
                Ok(v) => v,
                Err(e) => return Err(HkError::Parse {
                    line: line_num,
                    message: e.to_string(),
                }),
            };
            remaining = rest;
            line_num += 1;
            continue;
        }
        match section(remaining) {
            Ok((rest, (name, values))) => {
                config.insert(name, HkValue::Map(values));
                remaining = rest;
                line_num += count_lines(input) - count_lines(remaining) + 1; // Better approximation
            }
            Err(e) => {
                return Err(HkError::Parse {
                    line: line_num,
                    message: e.to_string(),
                });
            }
        }
    }
    Ok(config)
}
/// Counts the number of lines in a string (for error reporting).
fn count_lines(s: &str) -> usize {
    s.lines().count()
}
/// Loads and parses a .hk file from the given path.
pub fn load_hk_file<P: AsRef<Path>>(path: P) -> Result<HkConfig, HkError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut contents = String::new();
    for line in reader.lines() {
        contents.push_str(&line?);
        contents.push('\n');
    }
    parse_hk(&contents)
}
/// Serializes a HkConfig back to a .hk string.
pub fn serialize_hk(config: &HkConfig) -> String {
    let mut output = String::new();
    for (section, value) in config {
        output.push_str(&format!("[{}]\n", section));
        if let HkValue::Map(map) = value {
            serialize_map(map, 0, &mut output);
        }
        output.push('\n');
    }
    output.trim_end().to_string()
}
fn serialize_map(map: &HashMap<String, HkValue>, indent: usize, output: &mut String) {
    for (key, value) in map {
        match value {
            HkValue::String(s) => {
                output.push_str(&format!("{}-> {} => {}\n", " ".repeat(indent), key, s));
            }
            HkValue::Map(submap) => {
                output.push_str(&format!("{}-> {}\n", " ".repeat(indent), key));
                serialize_map(submap, indent + 1, output);
            }
        }
    }
}
/// Writes a HkConfig to a file.
pub fn write_hk_file<P: AsRef<Path>>(path: P, config: &HkConfig) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(serialize_hk(config).as_bytes())
}
// Parser combinators
fn comment<'a>(input: &'a str) -> IResult<&'a str, &'a str, nom::error::Error<&'a str>> {
    context(
        "comment",
        delimited(
            tag::<&str, &'a str, nom::error::Error<&'a str>>("!"),
                  not_line_ending::<&'a str, nom::error::Error<&'a str>>,
                  opt(newline::<&'a str, nom::error::Error<&'a str>>)
        ),
    )(input)
}
fn section<'a>(input: &'a str) -> IResult<&'a str, (String, HashMap<String, HkValue>), nom::error::Error<&'a str>> {
    context(
        "section",
        map(
            tuple((
                delimited(
                    tag::<&str, &'a str, nom::error::Error<&'a str>>("["),
                          take_until::<&str, &'a str, nom::error::Error<&'a str>>("]"),
                          tag::<&str, &'a str, nom::error::Error<&'a str>>("]")
                ),
                multispace0::<&'a str, nom::error::Error<&'a str>>,
                many0(alt((nested_key_value, key_value))),
            )),
            |(name, _, pairs)| {
                let mut map = HashMap::new();
                for (key, value) in pairs {
                    insert_nested(&mut map, key.split('.').collect::<Vec<_>>(), value);
                }
                (name.trim().to_string(), map)
            },
        ),
    )(input)
}
/// Inserts a value into a nested map using dot-separated keys.
fn insert_nested(map: &mut HashMap<String, HkValue>, keys: Vec<&str>, value: HkValue) {
    let mut current = map;
    for key in &keys[0..keys.len().saturating_sub(1)] {
        let entry = current.entry(key.to_string()).or_insert(HkValue::Map(HashMap::new()));
        if let HkValue::Map(submap) = entry {
            current = submap;
        } else {
            // Error if trying to nest under a string
            panic!("Invalid nesting");
        }
    }
    if let Some(last_key) = keys.last() {
        current.insert((*last_key).to_string(), value);
    }
}
fn key_value<'a>(input: &'a str) -> IResult<&'a str, (String, HkValue), nom::error::Error<&'a str>> {
    context(
        "key_value",
        map(
            tuple((
                preceded(
                    tuple((
                        multispace0::<&'a str, nom::error::Error<&'a str>>,
                        tag::<&str, &'a str, nom::error::Error<&'a str>>("->"),
                           multispace1::<&'a str, nom::error::Error<&'a str>>
                    )),
                    recognize(many1(satisfy(|c| c.is_alphanumeric() || c == '_')))
                ),
                multispace0::<&'a str, nom::error::Error<&'a str>>,
                tag::<&str, &'a str, nom::error::Error<&'a str>>("=>"),
                   multispace0::<&'a str, nom::error::Error<&'a str>>,
                   terminated(
                       not_line_ending::<&'a str, nom::error::Error<&'a str>>,
                       opt(newline::<&'a str, nom::error::Error<&'a str>>)
                   ),
            )),
            |(key, _, _, _, value)| (key.trim().to_string(), HkValue::String(value.trim().to_string())),
        ),
    )(input)
}
fn nested_key_value<'a>(input: &'a str) -> IResult<&'a str, (String, HkValue), nom::error::Error<&'a str>> {
    context(
        "nested_key_value",
        map(
            tuple((
                preceded(
                    tuple((
                        multispace0::<&'a str, nom::error::Error<&'a str>>,
                        tag::<&str, &'a str, nom::error::Error<&'a str>>("->"),
                           multispace1::<&'a str, nom::error::Error<&'a str>>
                    )),
                    recognize(many1(satisfy(|c| c.is_alphanumeric() || c == '_')))
                ),
                many1(sub_key_value),
            )),
            |(key, sub_pairs)| {
                let mut sub_map = HashMap::new();
                for (sub_key, sub_value) in sub_pairs {
                    sub_map.insert(sub_key, sub_value);
                }
                (key.trim().to_string(), HkValue::Map(sub_map))
            },
        ),
    )(input)
}
fn sub_key_value<'a>(input: &'a str) -> IResult<&'a str, (String, HkValue), nom::error::Error<&'a str>> {
    context(
        "sub_key_value",
        map(
            tuple((
                preceded(
                    tuple((
                        multispace1::<&'a str, nom::error::Error<&'a str>>,
                        tag::<&str, &'a str, nom::error::Error<&'a str>>("-->"),
                           multispace1::<&'a str, nom::error::Error<&'a str>>
                    )),
                    recognize(many1(satisfy(|c| c.is_alphanumeric() || c == '_')))
                ),
                multispace0::<&'a str, nom::error::Error<&'a str>>,
                tag::<&str, &'a str, nom::error::Error<&'a str>>("=>"),
                   multispace0::<&'a str, nom::error::Error<&'a str>>,
                   terminated(
                       not_line_ending::<&'a str, nom::error::Error<&'a str>>,
                       opt(newline::<&'a str, nom::error::Error<&'a str>>)
                   ),
            )),
            |(sub_key, _, _, _, sub_value)| (sub_key.trim().to_string(), HkValue::String(sub_value.trim().to_string())),
        ),
    )(input)
}
#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    #[test]
    fn test_parse_hk_with_comments() {
        let input = r#"
        ! Globalne informacje o projekcie
        [metadata]
        -> name => Hacker Lang
        -> version => 1.5
        -> authors => HackerOS Team <hackeros068@gmail.com>
        -> license => MIT
        [description]
        -> summary => Programing language for HackerOS.
        -> long => Język programowania Hacker Lang z plikami konfiguracyjnymi .hk lub .hacker lub skryptami itd. .hl.
        [specs]
        -> rust => >= 1.92.0
        -> dependencies
        --> odin => >= 2026-01
        --> c => C23
        --> crystal => 1.19.0
        --> python => 3.13
        "#;
        let result = parse_hk(input).unwrap();
        assert_eq!(result.len(), 3);
        if let Some(HkValue::Map(metadata)) = result.get("metadata") {
            assert_eq!(metadata.len(), 4);
            assert_eq!(metadata.get("name"), Some(&HkValue::String("Hacker Lang".to_string())));
            assert_eq!(metadata.get("version"), Some(&HkValue::String("1.5".to_string())));
            assert_eq!(metadata.get("authors"), Some(&HkValue::String("HackerOS Team <hackeros068@gmail.com>".to_string())));
            assert_eq!(metadata.get("license"), Some(&HkValue::String("MIT".to_string())));
        }
        if let Some(HkValue::Map(description)) = result.get("description") {
            assert_eq!(description.len(), 2);
            assert_eq!(description.get("summary"), Some(&HkValue::String("Programing language for HackerOS.".to_string())));
            assert_eq!(description.get("long"), Some(&HkValue::String("Język programowania Hacker Lang z plikami konfiguracyjnymi .hk lub .hacker lub skryptami itd. .hl.".to_string())));
        }
        if let Some(HkValue::Map(specs)) = result.get("specs") {
            assert_eq!(specs.len(), 2);
            assert_eq!(specs.get("rust"), Some(&HkValue::String(">= 1.92.0".to_string())));
            if let Some(HkValue::Map(deps)) = specs.get("dependencies") {
                assert_eq!(deps.len(), 4);
                assert_eq!(deps.get("odin"), Some(&HkValue::String(">= 2026-01".to_string())));
                assert_eq!(deps.get("c"), Some(&HkValue::String("C23".to_string())));
                assert_eq!(deps.get("crystal"), Some(&HkValue::String("1.19.0".to_string())));
                assert_eq!(deps.get("python"), Some(&HkValue::String("3.13".to_string())));
            }
        }
    }
    #[test]
    fn test_serialize_hk() {
        let mut config = HashMap::new();
        let mut metadata = HashMap::new();
        metadata.insert("name".to_string(), HkValue::String("Hacker Lang".to_string()));
        metadata.insert("version".to_string(), HkValue::String("1.5".to_string()));
        config.insert("metadata".to_string(), HkValue::Map(metadata));
        let serialized = serialize_hk(&config);
        assert!(serialized.contains("[metadata]"));
        assert!(serialized.contains("-> name => Hacker Lang"));
        assert!(serialized.contains("-> version => 1.5"));
    }
    #[test]
    fn test_error_handling() {
        let invalid_input = r#"
        [metadata]
        -> name = Hacker Lang # Missing =>
        "#;
        let err = parse_hk(invalid_input).unwrap_err();
        match err {
            HkError::Parse { line, message } => {
                assert_eq!(line, 1);
                assert!(message.contains("Parse error"));
            }
            _ => panic!("Unexpected error"),
        }
    }
}
