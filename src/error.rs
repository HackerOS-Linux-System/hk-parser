use colored::Colorize;
use std::io;
use thiserror::Error;

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
    /// Renders the error as a multi-line, rustc-style string: a boxed
    /// snippet of the surrounding source (when available) with a `^`
    /// caret under the exact column, plus a short "hint" for common
    /// mistakes. Returned as a `String` (rather than printed directly)
    /// so callers can log it, show it in a UI, write it to a file, or
    /// just print it — `pretty_print` below is a thin convenience
    /// wrapper around this that prints to stderr, kept for backwards
    /// compatibility with existing callers.
    pub fn render(&self, source: &str) -> String {
        let mut out = String::new();
        match self {
            Self::Parse { line, column, message } => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), message.bold()));
                out.push_str(&format!(
                    "  {} line {}, column {}\n",
                    "-->".blue().bold(),
                    line,
                    column
                ));

                render_snippet(&mut out, source, *line, *column);

                if let Some(hint) = hint_for(message) {
                    out.push_str(&format!("  {} {}\n", "hint:".yellow().bold(), hint.cyan()));
                }
            }
            Self::TypeMismatch { expected, found } => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), "type mismatch".bold()));
                out.push_str(&format!("  expected {}, found {}\n", expected.cyan(), found.red()));
            }
            Self::InvalidReference(reference) => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), "invalid reference".bold()));
                out.push_str(&format!("  {}\n", reference.red()));
                out.push_str(&format!(
                    "  {} the referenced key must exist and be reachable from the top of the file\n",
                    "hint:".yellow().bold()
                ));
            }
            Self::CyclicReference(path) => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), "cyclic reference".bold()));
                out.push_str(&format!("  {}\n", path.red()));
                out.push_str(&format!(
                    "  {} this key (transitively) refers back to itself through `${{...}}` interpolation\n",
                    "hint:".yellow().bold()
                ));
            }
            Self::KeyConflict(key) => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), "key conflict".bold()));
                out.push_str(&format!("  duplicate key '{}' in the same map\n", key.red()));
            }
            Self::MissingField(field) => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), "missing field".bold()));
                out.push_str(&format!("  '{}' is required but was not found\n", field.red()));
            }
            Self::Io(e) => {
                out.push_str(&format!("{} {}\n", "error:".red().bold(), "I/O error".bold()));
                out.push_str(&format!("  {}\n", e.to_string().red()));
            }
        }
        out
    }

    /// Prints `render(source)` to stderr. Kept for backwards compatibility
    /// with existing callers (e.g. hpm's own CLI) that expect this method
    /// to exist and print for them.
    pub fn pretty_print(&self, source: &str) {
        eprint!("{}", self.render(source));
    }
}

/// Appends a boxed source snippet (one line of context before/after the
/// error line when available, the error line itself, and a caret line)
/// to `out`. Column is 1-indexed and measured in `char`s, matching how
/// the parser computes it, so this handles non-ASCII lines correctly —
/// unlike the pre-3.2 version, which repeated `column` literal spaces
/// (a byte count) and drifted on any line with multi-byte characters
/// before the error column.
fn render_snippet(out: &mut String, source: &str, line: u32, column: usize) {
    if line == 0 {
        return;
    }
    let idx = (line - 1) as usize;
    let lines: Vec<&str> = source.lines().collect();
    let Some(err_line) = lines.get(idx) else {
        return;
    };

    // Width of the widest line-number gutter we'll print, so the `|`
    // separators line up even when going from e.g. line 9 to line 10.
    let gutter_width = line.to_string().len();

    let print_gutter_line = |num: Option<u32>, content: &str, out: &mut String| match num {
        Some(n) => out.push_str(&format!(
            "  {:>width$} {} {}\n",
            n.to_string().blue().bold(),
            "|".blue().bold(),
            content,
            width = gutter_width
        )),
        None => out.push_str(&format!(
            "  {:>width$} {}\n",
            "",
            "|".blue().bold(),
            width = gutter_width
        )),
    };

    if line > 1 {
        if let Some(prev) = lines.get(idx - 1) {
            print_gutter_line(Some(line - 1), prev, out);
        }
    }
    print_gutter_line(Some(line), err_line, out);

    // Caret line: one space per *character* (not byte) before the target
    // column, so it lands under the right character even with non-ASCII
    // text earlier on the line.
    let caret_offset = column.saturating_sub(1);
    let caret_line = format!("{}{}", " ".repeat(caret_offset), "^".red().bold());
    out.push_str(&format!(
        "  {:>width$} {} {}\n",
        "",
        "|".blue().bold(),
        caret_line,
        width = gutter_width
    ));

    if let Some(next) = lines.get(idx + 1) {
        print_gutter_line(Some(line + 1), next, out);
    }
}

/// Maps a parser error message to a short, actionable hint. Matched
/// against the exact messages `parse_hk`/`parse_map` actually produce
/// (see `src/parser.rs`) — earlier revisions of this function matched
/// nom-style fragments like `tag "=>"` that this hand-written parser
/// never emits, so no hint ever fired in practice.
fn hint_for(message: &str) -> Option<&'static str> {
    if message.contains("Expected key or map header") {
        Some("every non-blank, non-comment line must start with one or more '-' followed by '>', e.g. \"-> key => value\"")
    } else if message.contains("Expected '>' after dashes") {
        Some("dashes must be immediately followed by '>', e.g. \"-> key\" not \"- key\" or \"->key \"")
    } else if message.contains("Missing key after '>'") || message.contains("Empty key") || message.contains("Empty map key") {
        Some("write a key name right after '>', e.g. \"-> name => value\"")
    } else if message.contains("Inconsistent nesting level") {
        Some("nesting must increase by exactly one dash per level: '->', then '-->', then '--->' — don't skip a level")
    } else if message.contains("Unclosed array") {
        Some("every '[' that opens an array needs a matching ']' — check for a missing closing bracket")
    } else if message.contains("Unclosed section header") {
        Some("section headers need a closing ']', e.g. \"[metadata]\"")
    } else if message.contains("Empty section name") {
        Some("put a name between the brackets, e.g. \"[metadata]\" not \"[]\"")
    } else if message.contains("Expected section header") {
        Some("top-level content must start with a \"[section]\" header before any \"-> key => value\" lines")
    } else if message.contains("Empty value") {
        Some("there's nothing after '=>' — remove the key or give it a value")
    } else {
        None
    }
}
