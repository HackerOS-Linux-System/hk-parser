# hk-parser

A robust parser and serializer for HackerOS configuration files (.hk).

[![Crates.io](https://img.shields.io/crates/v/hk-parser.svg)](https://crates.io/crates/hk-parser)
[![Docs.rs](https://docs.rs/hk-parser/badge.svg)](https://docs.rs/hk-parser)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Overview

`hk-parser` is a Rust crate designed to parse and serialize configuration files in the `.hk` format, which is used in HackerOS. The `.hk` format is inspired by INI-like configurations but supports nested structures, comments, strong typing (strings, numbers, booleans, arrays, maps), interpolation (variables and references), and more advanced features like schema validation and derive macros for deserialization into Rust structs.

This crate aims to provide a professional, robust tool for handling configuration files with the following key features:
- **Strong Typing**: Support for multiple data types beyond just strings.
- **Interpolation**: Resolve environment variables and cross-references within the config.
- **Preserved Order**: Uses `IndexMap` to maintain the order of keys as read from the file.
- **Error Handling**: Detailed error messages with line and column information.
- **Derive Macro**: Easily deserialize `.hk` files into custom Rust structs using `#[derive(HkDeserialize)]`.
- **Serialization**: Serialize back to `.hk` format while preserving structure and order.
- **Validation**: (Planned) Schema-based validation using `.hks` files.

This README provides a comprehensive guide, including installation, usage examples, detailed explanations of features, API reference, and troubleshooting tips.

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [File Format (.hk)](#file-format-hk)
- [Features in Detail](#features-in-detail)
  - [Strong Typing](#strong-typing)
  - [Arrays](#arrays)
  - [Interpolation (Macros and Variables)](#interpolation-macros-and-variables)
  - [Preserved Key Order and Comments](#preserved-key-order-and-comments)
  - [Derive Macro for Deserialization](#derive-macro-for-deserialization)
  - [Validation and Schemas](#validation-and-schemas)
  - [Improved Parsing and Error Handling](#improved-parsing-and-error-handling)
- [API Reference](#api-reference)
- [Examples](#examples)
  - [Basic Parsing](#basic-parsing)
  - [Parsing with Types](#parsing-with-types)
  - [Interpolation Example](#interpolation-example)
  - [Serialization Example](#serialization-example)
  - [Derive Macro Example](#derive-macro-example)
  - [Error Handling Example](#error-handling-example)
- [Contributing](#contributing)
- [License](#license)
- [Changelog](#changelog)
- [FAQ](#faq)

## Installation

Add `hk-parser` to your `Cargo.toml`:

```toml
[dependencies]
hk-parser = "3.2.1"
```

If you need the derive macro, ensure your crate enables proc-macros (it's included by default).

For the latest version, check [crates.io](https://crates.io/crates/hk-parser).

## Quick Start

Here's a simple example to parse a `.hk` file:

```rust
use hk_parser::{load_hk_file, resolve_interpolations};
use std::path::Path;

fn main() -> Result<(), hk_parser::HkError> {
    let mut config = load_hk_file(Path::new("config.hk"))?;
    resolve_interpolations(&mut config)?;
    println!("{:?}", config);
    Ok(())
}
```

Example `config.hk`:

```
! Example configuration
[metadata]
-> name => hk-config
-> version => 1.5
-> active => true
-> pi => 3.14
-> authors => [ "Alice", "Bob" ]

[path]
-> bin => ${metadata.name}/bin
```

After parsing and resolving, `config` will have resolved values.

## File Format (.hk)

The `.hk` format is a human-readable configuration format with the following syntax:

- **Sections**: Defined in square brackets, e.g., `[metadata]`.
- **Key-Value Pairs**: `-> key => value` for top-level, `-->` for nested (but nesting can also be implied).
- **Nesting**: Supports dotted keys or indented sub-keys for nested maps.
- **Comments**: Lines starting with `!`.
- **Types**: Automatic detection for strings, numbers (f64), booleans (true/false), arrays `[item1, item2]`.
- **Interpolation**: `${var}` for env vars or references like `${section.key}`.
- **Arrays**: Inline arrays like `[1, 2.5, true, "str"]`.
- **Strings**: Can be quoted if containing special chars.

Example with all features:

```
! Advanced example with types and nesting
[metadata]
-> name => "HackerOS"
-> version => 4.9
-> is_active => true
-> constants
--> pi => 3.14159
--> e => 2.718
-> authors => ["HackerOS Team"]

[dependencies]
-> H-Sharp => "0.8"
-> others => ["Hacker Lang", "debian based", "Polish linux distro"]

[path]
-> home => ${HOME}
-> bin => ${metadata.name}/bin/${dependencies.rust}
```

## Features in Detail

### Strong Typing

The `HkValue` enum supports:
- `String(String)`
- `Number(f64)`
- `Bool(bool)`
- `Array(Vec<HkValue>)`
- `Map(IndexMap<String, HkValue>)`

During parsing, values are automatically typed: "true" becomes `Bool(true)`, "1.5" becomes `Number(1.5)`, etc.

Accessors like `value.as_string()`, `value.as_number()` return `Result` for type safety.

### Arrays

Two syntax styles are supported, and both produce the exact same `HkValue::Array`:

**Single-line** (comma-separated, classic style):

```
-> authors => ["Alice", "Bob"]
-> numbers => [1, 2, 3]
```

**Multi-line** (one item per line, since v3.2 — handy for longer lists like
tags, dependency lists, or filesystem paths, so you don't end up with a
150-character line):

```
-> tags => [
    "desktop"
    "environment"
    "gui"
]
```

A trailing comma per line is accepted but not required — `"desktop",` and
`"desktop"` on their own line both work, so you can freely reorder or
copy-paste lines without worrying about commas. An array item can itself be
another array (`[1, [2, 3], 4]`, on one line or spread across several) —
nesting is tracked by bracket depth, so commas *inside* a nested array no
longer split the *outer* array (fixed in v3.2; see [Changelog](#changelog)).

Items can be mixed types, including nested arrays.

Example parsing:

```rust
let array = config["metadata"]["authors"].as_array()?;
for item in array {
    println!("Author: {:?}", item.as_string()?);
}
```

### Interpolation (Macros and Variables)

After parsing, call `resolve_interpolations(&mut config)` to replace `${var}`:
- `${env:HOME}` for environment variables (prefix `env:` optional if not conflicting).
- `${section.key.subkey}` for cross-references.

Resolves recursively, handles cycles (but may error if infinite).

Example: See Quick Start.

### Preserved Key Order and Comments

Uses `indexmap` for `IndexMap` to keep insertion/read order during serialization.

Comments are not preserved in the data structure (yet), but serialization doesn't add/remove them. For full comment preservation, a future version may store them.

### Derive Macro for Deserialization

Use `#[derive(HkDeserialize)]` to map sections to structs.

Example:

```rust
#[derive(HkDeserialize)]
struct Metadata {
    name: String,
    version: f64,
    is_active: bool,
    authors: Vec<String>,
}

#[derive(HkDeserialize)]
struct Config {
    metadata: Metadata,
}

let config = load_hk_file("config.hk")?;
let struct_config = Config::from_hk_value(&HkValue::Map(config))?;
```

Supports `Option<T>` for optional fields.

### Validation and Schemas

(Planned feature) Use `.hks` schema files to validate required fields, types, semver, etc.

Example schema `.hks`:

```
[metadata]
-> version: semver
-> name: string required
```

Then `validate_hk(&config, load_hks("schema.hks"))?`.

Currently, manual validation via accessors.

### Improved Parsing and Error Handling

Hand-written line/column tracking parser for positioned errors: "Parse error at line X, column Y: message".

Supports trailing commas in arrays, multispace tolerance.

## API Reference

- `parse_hk(input: &str) -> Result<HkConfig, HkError>`: Parse from string.
- `load_hk_file(path: P) -> Result<HkConfig, HkError>`: Load from file.
- `resolve_interpolations(config: &mut HkConfig) -> Result<(), HkError>`: Resolve vars.
- `serialize_hk(config: &HkConfig) -> String`: Serialize to string.
- `write_hk_file(path: P, config: &HkConfig) -> io::Result<()>`: Write to file.
- `HkValue` enum with accessors.
- `HkError` for errors.
- `FromHkValue` trait for custom deserialization.
- `#[derive(HkDeserialize)]` macro.

Full docs at [docs.rs](https://docs.rs/hk-parser).

## Examples

### Basic Parsing

```rust
let input = r#"
[section]
-> key => value
"#;
let config = parse_hk(input)?;
assert_eq!(config["section"]["key"].as_string()?, "value");
```

### Parsing with Types

```rust
let input = r#"
[data]
-> num => 42.0
-> flag => false
-> list => [1, "two", true]
"#;
let config = parse_hk(input)?;
let num = config["data"]["num"].as_number()?; // 42.0
let list = config["data"]["list"].as_array()?; // Vec of HkValue
```

### Interpolation Example

```rust
let mut config = parse_hk(r#"
[info]
-> name => Test
[path]
-> dir => ${info.name}/dir
"#)?;
resolve_interpolations(&mut config)?;
assert_eq!(config["path"]["dir"].as_string()?, "Test/dir");
```

### Serialization Example

```rust
let mut config = IndexMap::new();
let mut section = IndexMap::new();
section.insert("key".to_string(), HkValue::String("value".to_string()));
config.insert("section".to_string(), HkValue::Map(section));
let serialized = serialize_hk(&config);
// [section]
// -> key => value
```

### Derive Macro Example

See above in Features.

### Error Handling Example

```rust
let invalid = r#"
[section]
-> key = value  ! Missing =>
"#;
if let Err(HkError::Parse { line, column, message }) = parse_hk(invalid) {
    println!("Error at line {}, col {}: {}", line, column, message);
}
```

### Nicer Error Output (since 3.2.1)

For a rustc-style boxed snippet with a caret and a hint instead of a bare
one-liner, use `render` (or `pretty_print`, which just prints `render`'s
output to stderr):

```rust
match parse_hk(&contents) {
    Ok(config) => { /* ... */ }
    Err(e) => {
        eprint!("{}", e.render(&contents)); // or: e.pretty_print(&contents);
        std::process::exit(1);
    }
}
```

```text
error: Expected key or map header
  --> line 4, column 1
  3 | -> ok => 1
  4 | oops
    | ^
  hint: every non-blank, non-comment line must start with one or more '-' followed by '>', e.g. "-> key => value"
```

## Contributing

Contributions welcome! Fork the repo, create a branch, submit a PR.

- Run tests: `cargo test`
- Build docs: `cargo doc --open`
- Issues: Report bugs or feature requests on GitHub.

Project layout (since 3.2.1 — previously one big `src/lib.rs`):

| File               | Contents                                              |
|--------------------|--------------------------------------------------------|
| `src/value.rs`     | `HkValue`, `HkConfig`                                  |
| `src/error.rs`     | `HkError`, `render`/`pretty_print`                      |
| `src/parser.rs`    | `parse_hk`, `load_hk_file`, everything they call        |
| `src/resolve.rs`   | `resolve_interpolations` (`${...}` interpolation)       |
| `src/serialize.rs` | `serialize_hk`, `write_hk_file`                          |
| `src/tests.rs`     | the test suite                                          |
| `src/lib.rs`       | module declarations + the crate-root `pub use` re-exports |

## License

MIT License. See [LICENSE](LICENSE).

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for the full history. Highlights of the
latest release:

### 3.2.1

- **Fixed:** multi-line arrays whose items were themselves arrays (e.g.
  `-> groups => [` / `    ["admins", "root"]` / `    ["users", "guest"]`
  / `]`) failed with `Unclosed array`. The section-boundary scan in
  `parse_hk` treated any `[`-led line as a brand new `[section]` header,
  even one that was really just an array item nested inside a still-open
  array value — so the section got cut off right after the opening `[`,
  before `parse_map` ever saw the closing `]`. The scan is now
  bracket-depth aware, matching the same logic already used for arrays
  themselves.
- **Improved:** error messages now render as a boxed, rustc-style
  snippet — a line or two of surrounding source, a gutter with line
  numbers, and a `^` caret under the exact column — via the new
  `HkError::render(&self, source: &str) -> String` method.
  `pretty_print` (unchanged signature) now just prints `render`'s
  output to stderr. The caret is now positioned by character count
  rather than byte count, so it no longer drifts on lines with
  non-ASCII text before the error column. Hints were also rewired to
  match the messages this parser actually produces (previous hints
  matched leftover nom-parser-style fragments like `tag "=>"` that
  never appeared in a real error).
- **Internal:** `src/lib.rs` split into `value.rs`, `error.rs`,
  `parser.rs`, `resolve.rs`, `serialize.rs`, and `tests.rs`. No public
  API changes — everything is still re-exported at the crate root
  exactly as before.

### 3.2.0

- **Added:** multi-line array syntax — `-> key => [` followed by one item
  per line and a closing `]`, as an alternative to the single-line
  `[a, b, c]` form. Trailing commas per line are optional.
- **Fixed:** nested arrays on a single line (`[1, [2, 3], 4]`) were
  previously split on *every* comma, including ones inside the nested
  array, corrupting the result. Comma-splitting is now bracket-depth
  aware.
- **Fixed:** every `Parse` error's reported `line` was off by one (too
  low) for anything inside a `[section]` — e.g. an error on the actual
  10th line of the file was reported as line 9. Root cause: an
  off-by-one in the line-number bookkeeping passed into the section's
  map parser. Column numbers and errors at the top level (bad/missing
  section headers) were unaffected.
- Quoted-string array items now keep their quotes through the internal
  tokenizer instead of losing them before the type-detection step, so
  escape sequences (`\n`, `\t`, `\"`, ...) inside array items decode
  correctly, matching how quoted strings already behaved outside
  arrays.

## FAQ

**Q: Why use IndexMap instead of HashMap?**  
A: To preserve key order from the file.

**Q: How to handle large files?**  
A: Parser is efficient, but for very large configs, consider streaming (future feature).

**Q: Can I preserve comments during serialization?**  
A: Not yet, but planned.

**Q: Integration with Serde?**  
A: Possible via custom serializers, but not built-in.

For more, see issues or contact HackerOS Team <hackeros068@gmail.com>.
