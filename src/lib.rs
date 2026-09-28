mod error;
mod ffi;
mod parser;
mod resolve;
mod serialize;
mod value;

pub use error::HkError;
pub use parser::{load_hk_file, parse_hk};
pub use resolve::resolve_interpolations;
pub use serialize::{serialize_hk, write_hk_file};
pub use value::{HkConfig, HkValue};

// `ffi`'s functions are all `#[no_mangle] extern "C"` — the C ABI itself
// is the public interface (see `src/bytes-io/main.h#`'s `extern dynamic
// [c, "hk_parser"]` block), so nothing from that module needs a `pub use`
// re-export here for other *Rust* code to see; declaring the module above
// is what makes `cargo build --release` (with `crate-type` including
// `"cdylib"` — see `Cargo.toml`) actually compile and export its symbols
// into `libhk_parser.so`.

#[cfg(test)]
mod tests;
