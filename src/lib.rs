mod error;
mod ffi;
#[cfg(feature = "lua")]
mod lua;
mod parser;
mod resolve;
mod serialize;
mod value;

pub use error::HkError;
pub use parser::{load_hk_file, parse_hk};
pub use resolve::{
    get_value_by_keys, get_value_by_path, parse_path, remove_value_by_path,
    remove_value_by_segments, resolve_interpolations, set_value_by_path, set_value_by_segments,
    PathSeg,
};
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
