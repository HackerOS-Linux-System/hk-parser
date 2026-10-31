use crate::error::HkError;
use crate::parser::{load_hk_file, parse_hk};
use crate::resolve::{
    get_value_by_keys, get_value_by_path, parse_path, remove_value_by_segments,
    resolve_interpolations, set_value_by_segments, PathSeg,
};
use crate::serialize::{serialize_hk, write_hk_file};
use crate::value::{HkConfig, HkValue};
use indexmap::IndexMap;
use mlua::prelude::*;
use mlua::{AnyUserData, MetaMethod, MultiValue, UserData, UserDataMethods, Value};

/// Hard stop for Lua → `.hk` conversion so a self-referencing table yields
/// an error instead of a stack overflow.
const MAX_DEPTH: usize = 128;

/// 2^53 — largest magnitude at which every integer is exactly representable
/// as an `f64`; above it a number stays a Lua float.
const MAX_SAFE_INT: f64 = 9_007_199_254_740_992.0;

// ─── Userdata ───────────────────────────────────────────────────────────────

/// A parsed document. Owns its data; freed by Lua's GC (no manual
/// `hk_free` needed, unlike the C ABI's handle registry).
struct LuaConfig(HkConfig);

// ─── Path argument ──────────────────────────────────────────────────────────

enum PathArg {
    Dotted(String),
    Keys(Vec<String>),
}

impl FromLua for PathArg {
    fn from_lua(value: Value, _lua: &Lua) -> LuaResult<Self> {
        match value {
            Value::String(s) => Ok(PathArg::Dotted(s.to_string_lossy().to_string())),
            Value::Table(t) => {
                let mut keys = Vec::new();
                for item in t.sequence_values::<LuaString>() {
                    keys.push(item?.to_string_lossy().to_string());
                }
                Ok(PathArg::Keys(keys))
            }
            other => Err(LuaError::runtime(format!(
                "path must be a string (\"a.b[0]\") or a list of keys ({{\"a\", \"b\"}}), got {}",
                other.type_name()
            ))),
        }
    }
}

impl PathArg {
    fn display(&self) -> String {
        match self {
            PathArg::Dotted(s) => s.clone(),
            PathArg::Keys(k) => k.join("."),
        }
    }

    fn segments(&self) -> Vec<PathSeg> {
        match self {
            PathArg::Dotted(s) => parse_path(s),
            PathArg::Keys(k) => k.iter().map(PathSeg::key).collect(),
        }
    }

    fn is_root(&self) -> bool {
        match self {
            PathArg::Dotted(s) => s.is_empty(),
            PathArg::Keys(k) => k.is_empty(),
        }
    }
}

fn lookup<'a>(cfg: &'a HkConfig, path: &PathArg) -> Option<&'a HkValue> {
    match path {
        PathArg::Dotted(s) if !s.is_empty() => get_value_by_path(s, cfg),
        PathArg::Keys(k) if !k.is_empty() => {
            let refs: Vec<&str> = k.iter().map(String::as_str).collect();
            get_value_by_keys(cfg, &refs)
        }
        _ => None,
    }
}

/// The map at `path` — the document root for `None`/`""`/`{}`.
fn map_at<'a>(cfg: &'a HkConfig, path: Option<&PathArg>) -> Option<&'a IndexMap<String, HkValue>> {
    match path {
        None => Some(cfg),
        Some(p) if p.is_root() => Some(cfg),
        Some(p) => lookup(cfg, p).and_then(|v| v.as_map().ok()),
    }
}

// ─── HkValue <-> Lua ────────────────────────────────────────────────────────

fn number_to_lua(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < MAX_SAFE_INT {
        Value::Integer(n as i64)
    } else {
        Value::Number(n)
    }
}

fn hk_to_lua(lua: &Lua, v: &HkValue) -> LuaResult<Value> {
    Ok(match v {
        HkValue::String(s) => Value::String(lua.create_string(s)?),
        HkValue::Number(n) => number_to_lua(*n),
        HkValue::Bool(b) => Value::Boolean(*b),
        HkValue::Array(a) => {
            let t = lua.create_table_with_capacity(a.len(), 0)?;
            for item in a {
                t.push(hk_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        HkValue::Map(m) => Value::Table(map_to_lua(lua, m)?),
    })
}

fn map_to_lua(lua: &Lua, m: &IndexMap<String, HkValue>) -> LuaResult<LuaTable> {
    let t = lua.create_table_with_capacity(0, m.len())?;
    for (k, v) in m {
        t.set(k.as_str(), hk_to_lua(lua, v)?)?;
    }
    Ok(t)
}

/// `Err(message)` rather than `LuaError` so callers can choose between
/// raising and the `nil, msg, info` convention.
fn lua_to_hk(v: &Value, depth: usize) -> Result<HkValue, String> {
    if depth > MAX_DEPTH {
        return Err(format!("table nesting deeper than {MAX_DEPTH} levels (cyclic table?)"));
    }
    match v {
        Value::Boolean(b) => Ok(HkValue::Bool(*b)),
        Value::Integer(i) => Ok(HkValue::Number(*i as f64)),
        Value::Number(n) => {
            if n.is_finite() {
                Ok(HkValue::Number(*n))
            } else {
                Err("NaN and infinity cannot be stored in a .hk file".to_string())
            }
        }
        Value::String(s) => Ok(HkValue::String(s.to_string_lossy().to_string())),
        Value::Table(t) => table_to_hk(t, depth),
        other => Err(format!("cannot store a Lua {} in a .hk file", other.type_name())),
    }
}

/// `true` when `t` is exactly the sequence `1..=n` with no other keys.
fn is_sequence(t: &LuaTable) -> bool {
    let n = t.raw_len();
    let mut count = 0usize;
    for pair in t.pairs::<Value, Value>() {
        match pair {
            Ok((Value::Integer(i), _)) if i >= 1 && (i as usize) <= n => count += 1,
            _ => return false,
        }
    }
    count == n
}

fn table_to_hk(t: &LuaTable, depth: usize) -> Result<HkValue, String> {
    if is_sequence(t) {
        let mut items = Vec::with_capacity(t.raw_len());
        for item in t.sequence_values::<Value>() {
            let item = item.map_err(|e| e.to_string())?;
            items.push(lua_to_hk(&item, depth + 1)?);
        }
        return Ok(HkValue::Array(items));
    }
    Ok(HkValue::Map(table_to_map(t, depth)?))
}

fn table_to_map(t: &LuaTable, depth: usize) -> Result<IndexMap<String, HkValue>, String> {
    let mut entries: Vec<(String, HkValue)> = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        let key = match k {
            Value::String(s) => s.to_string_lossy().to_string(),
            other => {
                return Err(format!(
                    "map keys must be strings, found a {} key (mixed or sparse array?)",
                    other.type_name()
                ))
            }
        };
        entries.push((key, lua_to_hk(&v, depth + 1)?));
    }
    // Lua gives no ordering guarantee — sort for reproducible output.
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries.into_iter().collect())
}

/// Top-level conversion: every entry must be a table (a section).
fn table_to_config(t: &LuaTable) -> Result<HkConfig, String> {
    let mut cfg: HkConfig = IndexMap::new();
    let mut entries: Vec<(String, LuaTable)> = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        let name = match k {
            Value::String(s) => s.to_string_lossy().to_string(),
            other => return Err(format!("section names must be strings, found a {} key", other.type_name())),
        };
        match v {
            Value::Table(section) => entries.push((name, section)),
            other => {
                return Err(format!(
                    "section '{name}' must be a table, found {}",
                    other.type_name()
                ))
            }
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, section) in entries {
        if section.raw_len() > 0 && is_sequence(&section) {
            return Err(format!("section '{name}' must be a map, found an array"));
        }
        cfg.insert(name, HkValue::Map(table_to_map(&section, 1)?));
    }
    Ok(cfg)
}

// ─── Error reporting ────────────────────────────────────────────────────────

fn kind_of(e: &HkError) -> &'static str {
    match e {
        HkError::Io(_) => "io",
        HkError::Parse { .. } => "parse",
        HkError::TypeMismatch { .. } => "type",
        HkError::MissingField(_) => "missing",
        HkError::InvalidReference(_) => "reference",
        HkError::CyclicReference(_) => "cycle",
        HkError::KeyConflict(_) => "conflict",
    }
}

/// `HkError::render` without ANSI colour codes (they'd show up as garbage
/// in a log file or a non-terminal).
fn render_plain(e: &HkError, source: &str) -> String {
    colored::control::set_override(false);
    let s = e.render(source);
    colored::control::unset_override();
    s
}

fn error_info(lua: &Lua, e: &HkError, source: Option<&str>) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("kind", kind_of(e))?;
    t.set("message", e.to_string())?;
    if let HkError::Parse { line, column, message } = e {
        t.set("line", *line)?;
        t.set("column", *column)?;
        t.set("detail", message.as_str())?;
        if let Some(src) = source {
            t.set("rendered", render_plain(e, src))?;
        }
    }
    Ok(t)
}

/// `nil, message, info`
fn fail(lua: &Lua, e: &HkError, source: Option<&str>) -> LuaResult<MultiValue> {
    let msg = Value::String(lua.create_string(e.to_string())?);
    let info = Value::Table(error_info(lua, e, source)?);
    Ok(MultiValue::from_iter([Value::Nil, msg, info]))
}

/// `nil, message, { kind = kind, message = message }` for conversion
/// failures that have no `HkError` of their own.
fn fail_msg(lua: &Lua, kind: &str, message: String) -> LuaResult<MultiValue> {
    let info = lua.create_table()?;
    info.set("kind", kind)?;
    info.set("message", message.as_str())?;
    Ok(MultiValue::from_iter([
        Value::Nil,
        Value::String(lua.create_string(&message)?),
        Value::Table(info),
    ]))
}

/// Like [`fail`], prefixing the message with the offending path.
fn fail_at(lua: &Lua, path: &PathArg, e: &HkError) -> LuaResult<MultiValue> {
    let kind = kind_of(e);
    fail_msg(lua, kind, format!("{}: {}", path.display(), e))
}

fn one(v: Value) -> LuaResult<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}

// ─── Construction ───────────────────────────────────────────────────────────

fn wants_resolve(opts: &Option<LuaTable>) -> LuaResult<bool> {
    match opts {
        Some(t) => Ok(t.get::<Option<bool>>("resolve")?.unwrap_or(false)),
        None => Ok(false),
    }
}

fn finish(
    lua: &Lua,
    parsed: Result<HkConfig, HkError>,
    resolve: bool,
    source: Option<&str>,
) -> LuaResult<MultiValue> {
    let mut cfg = match parsed {
        Ok(c) => c,
        Err(e) => return fail(lua, &e, source),
    };
    if resolve {
        if let Err(e) = resolve_interpolations(&mut cfg) {
            return fail(lua, &e, source);
        }
    }
    let ud = lua.create_userdata(LuaConfig(cfg))?;
    Ok(MultiValue::from_iter([Value::UserData(ud)]))
}

// ─── Methods ────────────────────────────────────────────────────────────────

impl UserData for LuaConfig {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // cfg:get(path [, default]) -> value | default | nil
        methods.add_method("get", |lua, this, (path, default): (PathArg, Value)| {
            match lookup(&this.0, &path) {
                Some(v) => hk_to_lua(lua, v),
                None => Ok(default),
            }
        });

        // cfg:has(path) -> boolean
        methods.add_method("has", |_, this, path: PathArg| Ok(lookup(&this.0, &path).is_some()));

        // cfg:get_string(path [, default]) -> string | default | nil
        //                                   | nil, msg, info   (arrays/maps)
        // Numbers and booleans are rendered as strings.
        methods.add_method(
            "get_string",
            |lua, this, (path, default): (PathArg, Option<LuaString>)| match lookup(&this.0, &path) {
                None => one(default.map(Value::String).unwrap_or(Value::Nil)),
                Some(v) => match v.as_string() {
                    Ok(s) => one(Value::String(lua.create_string(s)?)),
                    Err(e) => fail_at(lua, &path, &e),
                },
            },
        );

        // cfg:get_number(path [, default]) -> number | default | nil
        //                                   | nil, msg, info   (not a number)
        methods.add_method(
            "get_number",
            |lua, this, (path, default): (PathArg, Option<f64>)| match lookup(&this.0, &path) {
                None => one(default.map(number_to_lua).unwrap_or(Value::Nil)),
                Some(v) => match v.as_number() {
                    Ok(n) => one(number_to_lua(n)),
                    Err(e) => fail_at(lua, &path, &e),
                },
            },
        );

        // cfg:get_bool(path [, default]) -> boolean | default | nil
        //                                 | nil, msg, info   (not a boolean)
        methods.add_method(
            "get_bool",
            |lua, this, (path, default): (PathArg, Option<bool>)| match lookup(&this.0, &path) {
                None => one(default.map(Value::Boolean).unwrap_or(Value::Nil)),
                Some(v) => match v.as_bool() {
                    Ok(b) => one(Value::Boolean(b)),
                    Err(e) => fail_at(lua, &path, &e),
                },
            },
        );

        // cfg:type_of(path) -> "string"|"number"|"bool"|"array"|"map"|nil
        methods.add_method("type_of", |_, this, path: PathArg| {
            Ok(lookup(&this.0, &path).map(|v| v.type_name()))
        });

        // cfg:len([path]) -> element count of an array / key count of a map | nil
        methods.add_method("len", |_, this, path: Option<PathArg>| {
            if let Some(m) = map_at(&this.0, path.as_ref()) {
                return Ok(Some(m.len()));
            }
            Ok(path
                .as_ref()
                .and_then(|p| lookup(&this.0, p))
                .and_then(|v| v.as_array().ok())
                .map(|a| a.len()))
        });

        // cfg:keys([path]) -> { "key", ... } in file order | nil
        // No path (or "") lists the section names.
        methods.add_method("keys", |lua, this, path: Option<PathArg>| match map_at(&this.0, path.as_ref()) {
            None => Ok(Value::Nil),
            Some(m) => {
                let t = lua.create_table_with_capacity(m.len(), 0)?;
                for k in m.keys() {
                    t.push(k.as_str())?;
                }
                Ok(Value::Table(t))
            }
        });

        // cfg:to_table() -> plain nested Lua table (map order is lost)
        methods.add_method("to_table", |lua, this, ()| map_to_lua(lua, &this.0));

        // cfg:serialize() -> .hk text
        methods.add_method("serialize", |_, this, ()| Ok(serialize_hk(&this.0)));

        // cfg:write_file(path) -> true | nil, msg, info
        methods.add_method("write_file", |lua, this, path: LuaString| {
            let path = path.to_string_lossy().to_string();
            match write_hk_file(&path, &this.0) {
                Ok(()) => Ok(MultiValue::from_iter([Value::Boolean(true)])),
                Err(e) => fail(lua, &HkError::from(e), None),
            }
        });

        // cfg:clone() -> independent copy
        methods.add_method("clone", |_, this, ()| Ok(LuaConfig(this.0.clone())));

        // cfg:resolve() -> cfg | nil, msg, info   (expands ${...} in place)
        methods.add_function("resolve", |lua, ud: AnyUserData| {
            let result = {
                let mut this = ud.borrow_mut::<LuaConfig>()?;
                resolve_interpolations(&mut this.0)
            };
            match result {
                Ok(()) => Ok(MultiValue::from_iter([Value::UserData(ud)])),
                Err(e) => fail(lua, &e, None),
            }
        });

        // cfg:set(path, value) -> cfg | nil, msg, info
        //   value == nil deletes; missing sections/maps are created on the way.
        methods.add_function("set", |lua, (ud, path, value): (AnyUserData, PathArg, Value)| {
            let outcome: Result<(), (&'static str, String)> = {
                let mut this = ud.borrow_mut::<LuaConfig>()?;
                let segs = path.segments();
                if value.is_nil() {
                    remove_value_by_segments(&mut this.0, &segs);
                    Ok(())
                } else {
                    match lua_to_hk(&value, 0) {
                        Err(m) => Err(("type", m)),
                        Ok(hv) => set_value_by_segments(&mut this.0, &segs, hv)
                            .map_err(|e| (kind_of(&e), e.to_string())),
                    }
                }
            };
            match outcome {
                Ok(()) => one(Value::UserData(ud)),
                Err((kind, msg)) => fail_msg(lua, kind, format!("{}: {}", path.display(), msg)),
            }
        });

        // cfg:delete(path) -> boolean (whether something was removed)
        methods.add_method_mut("delete", |_, this, path: PathArg| {
            Ok(remove_value_by_segments(&mut this.0, &path.segments()).is_some())
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(serialize_hk(&this.0)));
        methods.add_meta_method(MetaMethod::Len, |_, this, ()| Ok(this.0.len()));
    }
}

// ─── Module entry point ─────────────────────────────────────────────────────

#[mlua::lua_module(name = "hk_parser_core")]
fn hk_parser_core(lua: &Lua) -> LuaResult<LuaTable> {
    let m = lua.create_table()?;
    m.set("version", env!("CARGO_PKG_VERSION"))?;

    // core.parse(text [, { resolve = bool }]) -> Config | nil, msg, info
    m.set(
        "parse",
        lua.create_function(|lua, (text, opts): (LuaString, Option<LuaTable>)| {
            let text = text.to_string_lossy().to_string();
            let resolve = wants_resolve(&opts)?;
            finish(lua, parse_hk(&text), resolve, Some(&text))
        })?,
    )?;

    // core.parse_file(path [, { resolve = bool }]) -> Config | nil, msg, info
    m.set(
        "parse_file",
        lua.create_function(|lua, (path, opts): (LuaString, Option<LuaTable>)| {
            let path = path.to_string_lossy().to_string();
            let resolve = wants_resolve(&opts)?;
            let parsed = load_hk_file(&path);
            // Best effort: re-read so a parse error can carry a source snippet.
            let source = if matches!(parsed, Err(HkError::Parse { .. })) {
                std::fs::read_to_string(&path).ok()
            } else {
                None
            };
            finish(lua, parsed, resolve, source.as_deref())
        })?,
    )?;

    // core.new() -> empty Config
    m.set("new", lua.create_function(|_, ()| Ok(LuaConfig(IndexMap::new())))?)?;

    // core.from_table({ section = { key = value, ... }, ... }) -> Config | nil, msg, info
    m.set(
        "from_table",
        lua.create_function(|lua, t: LuaTable| match table_to_config(&t) {
            Ok(cfg) => {
                let ud = lua.create_userdata(LuaConfig(cfg))?;
                Ok(MultiValue::from_iter([Value::UserData(ud)]))
            }
            Err(msg) => fail_msg(lua, "type", msg),
        })?,
    )?;

    Ok(m)
}
