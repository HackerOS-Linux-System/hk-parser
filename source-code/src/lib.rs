use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_while1},
    character::complete::{multispace0, multispace1, newline, not_line_ending},
    combinator::{map, opt},
    multi::{many0, many1},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

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

/// Parses a .hk file from a string input.
pub fn parse_hk(input: &str) -> Result<HkConfig, String> {
    match hk_file(input) {
        Ok((_, config)) => Ok(config),
        Err(e) => Err(format!("Parsing error: {}", e)),
    }
}

/// Loads and parses a .hk file from the given path.
pub fn load_hk_file<P: AsRef<Path>>(path: P) -> io::Result<HkConfig> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    parse_hk(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// Parser implementation using nom

fn hk_file(input: &str) -> IResult<&str, HkConfig> {
    let (input, sections) = many0(section)(input)?;
    let mut config = HashMap::new();
    for (name, values) in sections {
        config.insert(name, HkValue::Map(values));
    }
    Ok((input, config))
}

fn section(input: &str) -> IResult<&str, (String, HashMap<String, HkValue>)> {
    let (input, _) = multispace0(input)?;
    let (input, name) = delimited(tag("["), take_until("]"), tag("]"))(input)?;
    let (input, _) = multispace0(input)?;
    let (input, values) = many0(alt((nested_key_value, key_value)))(input)?;
    let mut map = HashMap::new();
    for (key, value) in values {
        map.insert(key, value);
    }
    Ok((input, (name.trim().to_string(), map)))
}

fn key_value(input: &str) -> IResult<&str, (String, HkValue)> {
    let (input, _) = multispace0(input)?;
    let (input, key) = terminated(take_while1(|c: char| c.is_alphanumeric() || c == '_'), tag(" => "))(input)?;
    let (input, value) = terminated(not_line_ending, opt(newline))(input)?;
    Ok((input, (key.trim().to_string(), HkValue::String(value.trim().to_string()))))
}

fn nested_key_value(input: &str) -> IResult<&str, (String, HkValue)> {
    let (input, _) = multispace0(input)?;
    let (input, key) = terminated(take_while1(|c: char| c.is_alphanumeric() || c == '_'), multispace0)(input)?;
    let (input, _) = tag("->")(input)?;
    let (input, sub_values) = many1(sub_key_value)(input)?;
    let mut sub_map = HashMap::new();
    for (sub_key, sub_value) in sub_values {
        sub_map.insert(sub_key, sub_value);
    }
    Ok((input, (key.trim().to_string(), HkValue::Map(sub_map))))
}

fn sub_key_value(input: &str) -> IResult<&str, (String, HkValue)> {
    let (input, _) = tuple((multispace1, tag("-->"), multispace0))(input)?;
    let (input, sub_key) = terminated(take_while1(|c: char| c.is_alphanumeric() || c == '_'), tag(" => "))(input)?;
    let (input, sub_value) = terminated(not_line_ending, opt(newline))(input)?;
    Ok((input, (sub_key.trim().to_string(), HkValue::String(sub_value.trim().to_string()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hk() {
        let input = r#"
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
            assert_eq!(metadata.get("name"), Some(&HkValue::String("Hacker Lang".to_string())));
            assert_eq!(metadata.get("version"), Some(&HkValue::String("1.5".to_string())));
        }

        if let Some(HkValue::Map(specs)) = result.get("specs") {
            assert_eq!(specs.get("rust"), Some(&HkValue::String(">= 1.92.0".to_string())));
            if let Some(HkValue::Map(deps)) = specs.get("dependencies") {
                assert_eq!(deps.get("odin"), Some(&HkValue::String(">= 2026-01".to_string())));
                assert_eq!(deps.get("c"), Some(&HkValue::String("C23".to_string())));
            }
        }
    }
}
