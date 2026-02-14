// src/lib.rs
//! Hacker Lang Configuration Parser
//!
//! This crate provides a robust parser and serializer for .hk files used in Hacker Lang.
//! It supports nested structures, comments, and error handling.

use indexmap::IndexMap;
use lazy_static::lazy_static;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_while, take_while1},
    character::complete::{multispace0, multispace1},
    combinator::{eof, map, opt, peek},
    error::{context, VerboseError, VerboseErrorKind},
    multi::{many0, many1, separated_list0},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};
use nom_locate::LocatedSpan;
use regex::Regex;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

type Span<'a> = LocatedSpan<&'a str>;
type ParseResult<'a, T> = IResult<Span<'a>, T, VerboseError<Span<'a>>>;

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
    /// Returns the value as a String.
    /// 
    /// FIX: Automatically converts Numbers and Bools to their string representation
    /// instead of returning a TypeMismatch error. This handles cases like 'version => 0.2'.
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
}

/// Parses a .hk file from a string input.
pub fn parse_hk(input: &str) -> Result<HkConfig, HkError> {
    let input_span = LocatedSpan::new(input);
    let mut remaining = input_span;
    let mut config = IndexMap::new();

    while !remaining.fragment().is_empty() {
        // Czyszczenie białych znaków i komentarzy przed parsowaniem sekcji
        let (rest, _) = many0(alt((
            multispace1,
            map(comment, |_| Span::new("")) 
        )))(remaining).map_err(|e| map_nom_error(input, remaining, e))?;
        
        remaining = rest;
        if remaining.fragment().is_empty() { break; }

        let (rest, (name, values)) = section(remaining).map_err(|e| map_nom_error(input, remaining, e))?;
        config.insert(name, HkValue::Map(values));
        remaining = rest;
    }

    Ok(config)
}

/// Helper to map nom error to HkError.
fn map_nom_error(input: &str, span: Span, err: nom::Err<VerboseError<Span>>) -> HkError {
    let verbose_err = match err {
        nom::Err::Error(e) | nom::Err::Failure(e) => e,
        nom::Err::Incomplete(_) => VerboseError { errors: vec![] },
    };
   
    let (line, column) = if let Some((s, _)) = verbose_err.errors.first() {
        (s.location_line(), s.get_column())
    } else {
        (span.location_line(), span.get_column())
    };

    let errors_str: Vec<(&str, VerboseErrorKind)> = verbose_err
        .errors
        .iter()
        .map(|(s, k)| (*s.fragment(), k.clone()))
        .collect();
    let verbose_err_str = VerboseError { errors: errors_str };
    let mut message = nom::error::convert_error(input, verbose_err_str);

    if message.contains("tag \"=>\"") {
        message.push_str("\nHint: Upewnij się, że po kluczu znajduje się '=>' przed wartością.");
    } else if message.contains("tag \"[\"") {
        message.push_str("\nHint: Sprawdź, czy sekcje zaczynają się od '[' i kończą ']'.");
    } else if message.contains("take_while1") {
        message.push_str("\nHint: Klucze mogą zawierać tylko litery, cyfry, '_', '-' i '.'.");
    }

    HkError::Parse {
        line,
        column,
        message,
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
   
    for (_, value) in config.iter_mut() {
        if let HkValue::Map(map) = value {
            resolve_map(map, &context)?;
        }
    }
    Ok(())
}

fn resolve_map(map: &mut IndexMap<String, HkValue>, top: &HkConfig) -> Result<(), HkError> {
    for (_, v) in map.iter_mut() {
        resolve_value(v, top)?;
    }
    Ok(())
}

fn resolve_value(v: &mut HkValue, top: &HkConfig) -> Result<(), HkError> {
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
                    resolve_path(var, top).ok_or(HkError::InvalidReference(var.to_string()))?
                };
                new_s.push_str(&repl);
                last = m.end();
            }
            new_s.push_str(&s[last..]);
            *s = new_s;
        }
        HkValue::Array(a) => {
            for item in a.iter_mut() {
                resolve_value(item, top)?;
            }
        }
        HkValue::Map(m) => resolve_map(m, top)?,
        _ => {}
    }
    Ok(())
}

fn resolve_path(path: &str, config: &HkConfig) -> Option<String> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current: Option<&HkValue> = config.get(parts[0]);
    for &p in &parts[1..] {
        current = current.and_then(|v| v.as_map().ok()).and_then(|m| m.get(p));
    }
    current.and_then(|v| v.as_string().ok())
}

/// Serializes a HkConfig back to a .hk string, preserving key order.
pub fn serialize_hk(config: &HkConfig) -> String {
    let mut output = String::new();
    for (section, value) in config.iter() {
        output.push_str(&format!("[{}]\n", section));
        if let HkValue::Map(map) = value {
            serialize_map(map, 0, &mut output);
        }
        output.push('\n');
    }
    output.trim_end().to_string()
}

fn serialize_map(map: &IndexMap<String, HkValue>, indent: usize, output: &mut String) {
    let spaces = " ".repeat(indent);
    for (key, value) in map.iter() {
        match value {
            HkValue::Map(submap) => {
                output.push_str(&format!("{}-> {}\n", spaces, key));
                serialize_map(submap, indent + 1, output);
            }
            _ => {
                output.push_str(&format!("{}-> {} => {}\n", spaces, key, serialize_value(value)));
            }
        }
    }
}

fn serialize_value(value: &HkValue) -> String {
    match value {
        HkValue::String(s) => {
            if s.contains(',') || s.contains(' ') || s.contains(']') || s.contains('"') {
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

// --- Parser Combinators ---

// Helper to define allowed characters in keys: alphanumeric, _, -, .
fn is_key_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn comment(input: Span) -> ParseResult<Span> {
    context(
        "comment",
        delimited(tag("!"), take_while(|c| c != '\r' && c != '\n'), opt(tag("\n"))),
    )(input)
}

fn section(input: Span) -> ParseResult<(String, IndexMap<String, HkValue>)> {
    context(
        "section",
        map(
            tuple((
                delimited(tag("["), take_until("]"), tag("]")),
                multispace0,
                terminated(
                    many0(alt((
                        map(comment, |_| None),
                        map(key_value, Some),
                        map(nested_key_value, Some),
                    ))),
                    // Using multispace0 ensures we consume trailing newlines before EOF or next section
                    tuple((multispace0, peek(alt((tag("["), map(eof, |_| Span::new(""))))))),
                ),
            )),
            |(name, _, opt_pairs)| {
                let mut map = IndexMap::new();
                for pair_opt in opt_pairs {
                    if let Some((key, value)) = pair_opt {
                        insert_nested(&mut map, key.split('.').collect::<Vec<_>>(), value);
                    }
                }
                (name.fragment().trim().to_string(), map)
            },
        ),
    )(input)
}

fn insert_nested(map: &mut IndexMap<String, HkValue>, keys: Vec<&str>, value: HkValue) {
    let mut current = map;
    for key in &keys[0..keys.len() - 1] {
        let entry = current
            .entry(key.to_string())
            .or_insert(HkValue::Map(IndexMap::new()));
        if let HkValue::Map(submap) = entry {
            current = submap;
        } else {
            // In a robust system, this might return an error rather than panic
            panic!("Invalid nesting: key conflict"); 
        }
    }
    if let Some(last_key) = keys.last() {
        current.insert(last_key.to_string(), value);
    }
}

fn key_value(input: Span) -> ParseResult<(String, HkValue)> {
    context(
        "key_value",
        map(
            tuple((
                preceded(
                    tuple((multispace0, tag("->"), multispace1)),
                    take_while1(is_key_char),
                ),
                multispace0,
                tag("=>"),
                line_value,
            )),
            |(key, _, _, value)| (key.fragment().trim().to_string(), value),
        ),
    )(input)
}

fn nested_key_value(input: Span) -> ParseResult<(String, HkValue)> {
    context(
        "nested_key_value",
        map(
            tuple((
                preceded(
                    // multispace0 here allows for "compressed" lists or standard spacing
                    tuple((multispace0, tag("->"), multispace1)),
                    take_while1(is_key_char),
                ),
                many1(sub_key_value),
            )),
            |(key, sub_pairs)| {
                let mut sub_map = IndexMap::new();
                for (sub_key, sub_value) in sub_pairs {
                    sub_map.insert(sub_key, sub_value);
                }
                (key.fragment().trim().to_string(), HkValue::Map(sub_map))
            },
        ),
    )(input)
}

fn sub_key_value(input: Span) -> ParseResult<(String, HkValue)> {
    context(
        "sub_key_value",
        map(
            tuple((
                preceded(
                    // FIX: Changed multispace1 to multispace0. 
                    // line_value consumes the newline. If there is no indentation, 
                    // multispace1 fails because there is no whitespace left.
                    tuple((multispace0, tag("-->"), multispace1)),
                    take_while1(is_key_char),
                ),
                multispace0,
                tag("=>"),
                line_value,
            )),
            |(sub_key, _, _, sub_value)| (sub_key.fragment().trim().to_string(), sub_value),
        ),
    )(input)
}

fn line_value(input: Span) -> ParseResult<HkValue> {
    preceded(
        multispace0,
        alt((
            map(array, HkValue::Array),
            map(
                // Consumes until newline, and optionally consumes the newline itself
                terminated(
                    take_while(|c| c != '\r' && c != '\n'), 
                    opt(tag("\n"))
                ),
                |s: Span| parse_simple(s.fragment()),
            ),
        )),
    )(input)
}

fn parse_simple(s: &str) -> HkValue {
    let s = s.trim();
    if s.eq_ignore_ascii_case("true") {
        HkValue::Bool(true)
    } else if s.eq_ignore_ascii_case("false") {
        HkValue::Bool(false)
    } else if let Ok(n) = f64::from_str(s) {
        HkValue::Number(n)
    } else {
        HkValue::String(s.to_string())
    }
}

fn array(input: Span) -> ParseResult<Vec<HkValue>> {
    delimited(
        tag("["),
        separated_list0(tuple((multispace0, tag(","), multispace0)), item_value),
        tag("]"),
    )(input)
    .map(|(i, v)| (i, v))
}

fn item_value(input: Span) -> ParseResult<HkValue> {
    alt((
        map(array, HkValue::Array),
        map(double_quoted, |s| HkValue::String(s.fragment().to_string())),
        map(
            take_while1(|c: char| !c.is_whitespace() && c != ',' && c != ']'),
            |s: Span| parse_simple(s.fragment()),
        ),
    ))(input)
}

fn double_quoted(input: Span) -> ParseResult<Span> {
    delimited(tag("\""), take_while(|c| c != '"'), tag("\""))(input)
}

// --- Type Conversion Traits ---

pub trait FromHkValue: Sized {
    fn from_hk_value(value: &HkValue) -> Result<Self, HkError>;
}

impl FromHkValue for String {
    fn from_hk_value(value: &HkValue) -> Result<Self, HkError> {
        value.as_string()
    }
}

impl FromHkValue for f64 {
    fn from_hk_value(value: &HkValue) -> Result<Self, HkError> {
        value.as_number()
    }
}

impl FromHkValue for bool {
    fn from_hk_value(value: &HkValue) -> Result<Self, HkError> {
        value.as_bool()
    }
}

impl<T: FromHkValue> FromHkValue for Vec<T> {
    fn from_hk_value(value: &HkValue) -> Result<Self, HkError> {
        value
            .as_array()?
            .iter()
            .map(|v| T::from_hk_value(v))
            .collect()
    }
}

impl<T: FromHkValue> FromHkValue for Option<T> {
    fn from_hk_value(value: &HkValue) -> Result<Self, HkError> {
        Ok(Some(T::from_hk_value(value)?))
    }
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
        
        if let Some(HkValue::Map(libraries)) = result.get("libraries") {
            // Check obsidian
            if let Some(HkValue::Map(obsidian)) = libraries.get("obsidian") {
                // Internal representation is Number
                assert_eq!(obsidian.get("version"), Some(&HkValue::Number(0.2)));
                // But as_string() should convert it gracefully now
                assert_eq!(obsidian.get("version").unwrap().as_string().unwrap(), "0.2");
                
                assert_eq!(obsidian.get("description").unwrap().as_string().unwrap(), "Biblioteka inspirowana zenity.");
                assert!(obsidian.contains_key("so-download"));
                assert!(obsidian.contains_key(".hl-download"));
            } else {
                panic!("Missing obsidian key");
            }

            // Check yuy
             if let Some(HkValue::Map(yuy)) = libraries.get("yuy") {
                assert_eq!(yuy.get("version"), Some(&HkValue::Number(0.2)));
            } else {
                panic!("Missing yuy key");
            }
        } else {
            panic!("Missing libraries section");
        }
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
    }
}
