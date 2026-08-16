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

#[test]
fn test_skipped_nesting_level_is_rejected() {
    // Reproduces the exact "logging" bug: a level-2 map header ("-->")
    // whose children jump straight to level 4 ("---->"), skipping level 3.
    // Previously this silently produced an empty map instead of erroring.
    let input = r#"
[global]
-> features
--> logging
----> level => debug
----> file => /var/log/app.log
"#;
    let result = parse_hk(input);
    assert!(result.is_err(), "Expected a parse error for skipped nesting level, got Ok");
    match result.unwrap_err() {
        HkError::Parse { message, .. } => {
            assert!(message.contains("Inconsistent nesting level"), "unexpected message: {message}");
        }
        other => panic!("Expected HkError::Parse, got {:?}", other),
    }
}

#[test]
fn test_skipped_nesting_level_in_dotted_key_header_is_rejected() {
    // Reproduces the "array.of.objects" bug: a level-1 dotted-key map header
    // whose children jump straight to level 3 ("--->"), skipping level 2.
    let input = r#"
[deep]
-> array.of.objects
---> item => value
"#;
    let result = parse_hk(input);
    assert!(result.is_err(), "Expected a parse error for skipped nesting level, got Ok");
}

#[test]
fn test_correct_incremental_nesting_still_works() {
    // Sanity check: properly incremented nesting (one extra dash per level)
    // must keep working exactly as before.
    let input = r#"
[global]
-> features
--> logging
---> level => debug
---> file => /var/log/app.log
"#;
    let config = parse_hk(input).unwrap();
    let logging = config["global"]
        .as_map().unwrap()["features"]
        .as_map().unwrap()["logging"]
        .as_map().unwrap();
    assert_eq!(logging["level"].as_string().unwrap(), "debug");
    assert_eq!(logging["file"].as_string().unwrap(), "/var/log/app.log");
}

#[test]
fn test_shallower_return_to_caller_still_works() {
    // Sanity check: legitimately dedenting back to a shallower level
    // (the normal "end of this nested map" case) must NOT error.
    let input = r#"
[a]
-> x
--> y => 1
-> z => 2
"#;
    let config = parse_hk(input).unwrap();
    let a = config["a"].as_map().unwrap();
    assert_eq!(a["x"].as_map().unwrap()["y"].as_number().unwrap(), 1.0);
    assert_eq!(a["z"].as_number().unwrap(), 2.0);
}

#[test]
fn test_empty_string_roundtrip() {
    // An empty string must serialize back to a quoted "" so it can be
    // re-parsed, instead of silently disappearing.
    let input = r#"
[a]
-> key => ""
"#;
    let config = parse_hk(input).unwrap();
    assert_eq!(config["a"].as_map().unwrap()["key"].as_string().unwrap(), "");
    let serialized = serialize_hk(&config);
    assert!(serialized.contains("\"\""), "expected empty string to serialize as \"\", got: {serialized}");
    let parsed_again = parse_hk(&serialized).unwrap();
    assert_eq!(config, parsed_again);
}

// -----------------------------------------------------------------
// v3.2: multi-line arrays
// -----------------------------------------------------------------

#[test]
fn test_multiline_array_basic() {
    // The exact style hpm's example packages use for `tags`, `filesystem`,
    // `deb_deps`, etc. — one bare item per line, no commas required.
    let input = r#"
[metadata]
-> tags => [
    "desktop"
    "environment"
    "gui"
]
"#;
    let config = parse_hk(input).unwrap();
    let tags = config["metadata"].as_map().unwrap()["tags"].as_array().unwrap();
    let tags: Vec<String> = tags.iter().map(|v| v.as_string().unwrap()).collect();
    assert_eq!(tags, vec!["desktop", "environment", "gui"]);
}

#[test]
fn test_multiline_array_with_trailing_commas() {
    let input = r#"
[a]
-> list => [
    "one",
    "two",
    "three",
]
"#;
    let config = parse_hk(input).unwrap();
    let list = config["a"].as_map().unwrap()["list"].as_array().unwrap();
    let list: Vec<String> = list.iter().map(|v| v.as_string().unwrap()).collect();
    assert_eq!(list, vec!["one", "two", "three"]);
}

#[test]
fn test_multiline_array_empty() {
    let input = r#"
[a]
-> list => [
]
-> after => "still parses"
"#;
    let config = parse_hk(input).unwrap();
    let a = config["a"].as_map().unwrap();
    assert_eq!(a["list"].as_array().unwrap().len(), 0);
    assert_eq!(a["after"].as_string().unwrap(), "still parses");
}

#[test]
fn test_multiline_array_mixed_types_and_key_after() {
    // Make sure parsing correctly resumes with the next key after a
    // multi-line array closes (this is what line 24's error was really
    // about: the parser losing track and treating the next array item
    // line as if it were a fresh "-> key" line).
    let input = r#"
[data]
-> numbers => [
    1
    2.5
    true
    "four"
]
-> next_key => "reached"
"#;
    let config = parse_hk(input).unwrap();
    let data = config["data"].as_map().unwrap();
    let numbers = data["numbers"].as_array().unwrap();
    assert_eq!(numbers.len(), 4);
    assert_eq!(numbers[0].as_number().unwrap(), 1.0);
    assert_eq!(numbers[1].as_number().unwrap(), 2.5);
    assert_eq!(numbers[2].as_bool().unwrap(), true);
    assert_eq!(numbers[3].as_string().unwrap(), "four");
    assert_eq!(data["next_key"].as_string().unwrap(), "reached");
}

#[test]
fn test_nested_array_single_line_comma_depth() {
    // Regression check: a comma INSIDE a nested array must not split
    // the outer array (this was broken pre-3.2 — the old scanner had
    // no bracket-depth tracking around commas).
    let input = r#"
[a]
-> matrix => [1, [2, 3], 4]
"#;
    let config = parse_hk(input).unwrap();
    let matrix = config["a"].as_map().unwrap()["matrix"].as_array().unwrap();
    assert_eq!(matrix.len(), 3);
    assert_eq!(matrix[0].as_number().unwrap(), 1.0);
    let inner = matrix[1].as_array().unwrap();
    assert_eq!(inner.len(), 2);
    assert_eq!(inner[0].as_number().unwrap(), 2.0);
    assert_eq!(inner[1].as_number().unwrap(), 3.0);
    assert_eq!(matrix[2].as_number().unwrap(), 4.0);
}

#[test]
fn test_nested_array_multiline() {
    // FIXED (v3.2.1): this used to fail with "Unclosed array" because
    // parse_hk's section-boundary scan saw `["admins", "root"]` (an
    // array item that is itself an array) starting with '[' and mistook
    // it for a brand new `[section]` header, cutting the section off
    // right after `-> groups => [` — see the comment on that scan in
    // src/parser.rs for the full explanation.
    let input = r#"
[a]
-> groups => [
    ["admins", "root"]
    ["users", "guest"]
]
"#;
    let config = parse_hk(input).unwrap();
    let groups = config["a"].as_map().unwrap()["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    let g0 = groups[0].as_array().unwrap();
    assert_eq!(g0[0].as_string().unwrap(), "admins");
    assert_eq!(g0[1].as_string().unwrap(), "root");
    let g1 = groups[1].as_array().unwrap();
    assert_eq!(g1[0].as_string().unwrap(), "users");
    assert_eq!(g1[1].as_string().unwrap(), "guest");
}

#[test]
fn test_nested_array_multiline_followed_by_another_section() {
    // Same bug as above, but also checks that a real section header
    // *after* the array is still found correctly (i.e. the fix doesn't
    // just get lucky because the array happened to be the last thing
    // in the file).
    let input = r#"
[a]
-> groups => [
    ["admins", "root"]
    ["users", "guest"]
]

[b]
-> ok => true
"#;
    let config = parse_hk(input).unwrap();
    assert_eq!(config["a"].as_map().unwrap()["groups"].as_array().unwrap().len(), 2);
    assert_eq!(config["b"].as_map().unwrap()["ok"].as_bool().unwrap(), true);
}

#[test]
fn test_multiline_array_unclosed_is_a_clean_error() {
    let input = r#"
[a]
-> list => [
    "one"
"#;
    let result = parse_hk(input);
    assert!(result.is_err());
    match result.unwrap_err() {
        HkError::Parse { message, .. } => {
            assert!(message.contains("Unclosed array"), "unexpected message: {message}");
        }
        other => panic!("Expected HkError::Parse, got {:?}", other),
    }
}

#[test]
fn test_multiline_array_roundtrips_through_serialize() {
    let input = r#"
[a]
-> tags => [
    "x"
    "y"
]
"#;
    let config = parse_hk(input).unwrap();
    let serialized = serialize_hk(&config);
    // serialize_hk always emits the single-line style — that's fine,
    // both styles must parse back to the identical HkConfig.
    let parsed_again = parse_hk(&serialized).unwrap();
    assert_eq!(config, parsed_again);
}

// -----------------------------------------------------------------
// v3.2: off-by-one line number fix
// -----------------------------------------------------------------

#[test]
fn test_error_line_number_is_accurate() {
    // Line 1 is blank (raw string literal starts with \n), line 2 is
    // "[a]", line 3 is a valid field, line 4 has no leading dash at
    // all — that's the one that must be reported, not line 3.
    // (Pre-3.2 this incorrectly reported line 3.)
    let input = "\n[a]\n-> ok => 1\noops\n";
    let result = parse_hk(input);
    match result {
        Err(HkError::Parse { line, message, .. }) => {
            assert_eq!(line, 4, "expected the error on line 4 (1-indexed), got {line}");
            assert!(message.contains("Expected key or map header"));
        }
        other => panic!("Expected a Parse error, got {:?}", other),
    }
}

// -----------------------------------------------------------------
// v3.2.1: nicer error rendering
// -----------------------------------------------------------------

#[test]
fn test_render_includes_snippet_and_hint() {
    let input = "\n[a]\noops\n";
    let err = parse_hk(input).unwrap_err();
    let rendered = err.render(input);
    // Strip ANSI color codes for a stable substring check regardless of
    // whether the `colored` crate's auto-detection decides to colorize
    // in the test environment.
    let plain: String = strip_ansi(&rendered);
    assert!(plain.contains("line 3"), "rendered output missing line number:\n{plain}");
    assert!(plain.contains("oops"), "rendered output missing the offending source line:\n{plain}");
    assert!(plain.contains('^'), "rendered output missing a caret:\n{plain}");
    assert!(plain.contains("hint:"), "rendered output missing a hint:\n{plain}");
}

#[test]
fn test_render_handles_first_line_error_without_panicking() {
    // line == 1 means there's no "previous line" to show — must not panic
    // on the `idx - 1` underflow that a naive implementation would hit.
    let input = "oops\n";
    let err = parse_hk(input).unwrap_err();
    let _ = err.render(input); // must not panic
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip the CSI sequence: '[' then digits/semicolons, then a
            // final letter (commonly 'm' for SGR/color codes).
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
