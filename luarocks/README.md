# hk-parser — Lua bindings (LuaRocks)

Parse, query, edit and write HackerOS `.hk` configuration files from Lua. The
parser is the Rust crate in this repository, compiled into a native module
(`hk_parser_core`, source: `../src/lua.rs`, built with
[mlua](https://crates.io/crates/mlua)); `lua/hk_parser.lua` is the small
pure-Lua API you `require`.

Tested on Lua 5.1, 5.2, 5.3, 5.4 and LuaJIT (Linux). `lua55` is wired up but
not tested; macOS (`build.rs` adds the linker flags) and Windows are untested.

## Layout

```
luarocks/
├── hk-parser-scm-1.rockspec     dev rockspec (git main)
├── hk-parser-0.3.2-1.rockspec   release rockspec (needs a v0.3.2 git tag)
├── lua/hk_parser.lua            the facade: require "hk_parser"
├── test/run.lua                 self-contained test-suite (45 tests, no busted)
└── examples/basic.lua
```

## Install

Needs a Rust toolchain (`cargo`, a recent one — the dependency tree wants
edition 2024 support, i.e. Rust ≥ 1.85) at install time.

```sh
luarocks install luarocks-build-rust-mlua
# from a checkout (repo root):
luarocks --local make luarocks/hk-parser-scm-1.rockspec
luarocks --local test luarocks/hk-parser-scm-1.rockspec     # optional
```

Without LuaRocks (module only, into `dist/lua/`):

```sh
hl build.hl -- lua-test            # Lua 5.4: build + run the tests
hl build.hl -- lua-test luajit     # or lua51 lua52 lua53 lua55 luajit
# by hand:
cargo build --release --lib --features lua54
cp target/release/libhk_parser.so hk_parser_core.so
LUA_PATH='luarocks/lua/?.lua' LUA_CPATH='./?.so' lua5.4 luarocks/test/run.lua
```

## Usage

```lua
local hk = require("hk_parser")

local cfg, err, info = hk.load("config.hk", { resolve = true })
if not cfg then
  io.stderr:write(info.rendered or err, "\n")
  os.exit(1)
end

cfg:get_string("global.name")              --> "Hacker Lang Parser Demo"
cfg:get_number("global.database.port", 5432)
cfg:get("global.tags[0]")                  --> first array element (0-based!)
for key, value in cfg:each("global.database") do ... end   -- file order

cfg:set("global.version", 3):delete("global.production")
assert(cfg:write_file("config.hk"))
```

## API

**Module `hk_parser`**

| function | returns |
|---|---|
| `parse(text [, opts])` / `parse_file(path [, opts])` (= `load`) | `Config` or `nil, err, info` |
| `new()` | empty `Config` |
| `from_table(tbl)` | `Config` or `nil, err, info` |
| `decode(text [, opts])` / `decode_file(path [, opts])` | plain table or `nil, err, info` |
| `encode(tbl)` | `.hk` text or `nil, err, info` |
| `encode_file(tbl, path)` | `true` or `nil, err, info` |
| `is_config(x)`, `VERSION`, `core` (raw native module) | |

`opts = { resolve = true }` expands `${section.key}` / `${env:NAME}` immediately.

**`Config` methods**

| method | notes |
|---|---|
| `get(path [, default])` | any value; maps/arrays become plain tables |
| `get_string / get_number / get_bool (path [, default])` | `default` (or `nil`) when absent; **raises** if present with another type. `get_string` renders numbers/booleans as text |
| `expect(path)` | like `get`, raises `missing field: …` when absent |
| `has(path)`, `type_of(path)` | `type_of` → `"string" "number" "bool" "array" "map"` or `nil` |
| `keys([path])`, `sections()`, `len([path])` | file order; no path = sections |
| `each([path])` | iterator: `key, value` for maps (file order), `index, value` for arrays |
| `to_table()` | whole document as plain tables (map order is lost) |
| `set(path, value)` | creates sections/maps as needed, keeps position of existing keys, `nil` deletes; chainable; raises if impossible |
| `delete(path)` | `true` if something was removed |
| `resolve()` | expand interpolations in place → `self` or `nil, err, info` |
| `clone()`, `serialize()` (= `tostring(cfg)`), `write_file(path)` | |

### Paths

`"section.key"`, `"section.list[2]"` — array indices are **0-based**, exactly
as in the `.hk` format (`${arrays.numbers[0]}`); the Lua tables you get back
are ordinary 1-based. For keys that contain `.` or brackets (e.g. the
`.hl-download` key of the libraries format) pass a list: `{"libs", ".hl-download"}`.
Note the parser itself expands dotted keys in a *file* (`-> a.b => 1` becomes
`a` → `b`).

### Errors

* Data problems (syntax errors, unreadable files, cyclic or invalid `${…}`)
  return `nil, message, info` — use `assert(...)` to raise instead.
  `info = { kind, message [, line, column, detail, rendered] }`, `kind` is one
  of `parse io type missing reference cycle conflict`; for `parse` errors
  `rendered` is a rustc-style snippet with a caret (no ANSI colours).
* Programmer mistakes (wrong argument types, typed getter on a value of
  another type, impossible `set`) raise ordinary string errors positioned at
  your call site.

### Types and ordering

* `.hk` numbers are `f64`; integral values arrive as Lua integers on 5.3+
  (`port => 8080` → `8080`, not `8080.0`).
* Lua → `.hk`: a table with keys `1..n` is an array, anything else is a map
  whose keys are written **alphabetically** (Lua tables are unordered); an
  empty table is an empty array. Build with `new()` + `set()` when order
  matters. NaN/inf, functions and cyclic tables are rejected.
* `#cfg` and `pairs(cfg)` (sections) need Lua ≥ 5.2 / LuaJIT 5.2-compat; use
  `cfg:len()` / `cfg:each()` everywhere.

## Publishing to luarocks.org

1. tag the release (`git tag v0.3.2 && git push --tags`), keep the version in
   `Cargo.toml` and `hk-parser-<ver>-1.rockspec` in sync;
2. `luarocks upload luarocks/hk-parser-0.3.2-1.rockspec --api-key=…`.
