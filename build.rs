fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_LUA");

    let lua_enabled = std::env::var_os("CARGO_FEATURE_LUA").is_some();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if lua_enabled && target_os == "macos" {
        println!("cargo:rustc-cdylib-link-arg=-undefined");
        println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
    }
}
