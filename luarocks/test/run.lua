local hk = require("hk_parser")

local passed, failed = 0, 0
local failures = {}

local function test(name, fn)
  local ok, err = xpcall(fn, function(e) return debug.traceback(tostring(e), 2) end)
  if ok then
    passed = passed + 1
    io.write(".")
  else
    failed = failed + 1
    failures[#failures + 1] = { name = name, err = err }
    io.write("F")
  end
end

local function deep_equal(a, b)
  if type(a) ~= type(b) then return false end
  if type(a) ~= "table" then return a == b end
  for k, v in pairs(a) do
    if not deep_equal(v, b[k]) then return false end
  end
  for k in pairs(b) do
    if a[k] == nil then return false end
  end
  return true
end

local function show(v)
  if type(v) ~= "table" then return tostring(v) end
  local parts = {}
  for k, x in pairs(v) do parts[#parts + 1] = tostring(k) .. "=" .. show(x) end
  table.sort(parts)
  return "{" .. table.concat(parts, ", ") .. "}"
end

local function eq(actual, expected, msg)
  if not deep_equal(actual, expected) then
    error((msg or "values differ") .. ": expected " .. show(expected) .. ", got " .. show(actual), 2)
  end
end

local function truthy(v, msg)
  if not v then error(msg or "expected a truthy value", 2) end
end

--- `fn` must raise a *string* error matching `pattern` (plain find).
local function raises(fn, pattern, msg)
  local ok, err = pcall(fn)
  if ok then error((msg or "expected an error") .. " but nothing was raised", 2) end
  if type(err) ~= "string" then
    error((msg or "error must be a string") .. ", got " .. type(err) .. ": " .. tostring(err), 2)
  end
  if pattern and not err:find(pattern, 1, true) then
    error((msg or "wrong error") .. ": expected it to contain '" .. pattern .. "', got '" .. err .. "'", 2)
  end
  return err
end

local SAMPLE = [[
! demo
[global]
-> name => "Hacker Lang Parser Demo"
-> version => 2.0
-> port => 8080
-> pi => 3.14159
-> production => true
-> tags => ["rust", "parser", "hk-format"]
-> database
--> host => localhost
--> port => 5432
--> credentials
---> username => admin

[arrays]
-> numbers => [1, 2, 3]
-> mixed => [42, "text", true, 3.14]
-> multi => [
    "a"
    "b"
]

[refs]
-> from_global => ${global.name}
-> first_number => ${arrays.numbers[0]}
-> home => ${env:PATH}
-> composed => ${global.database.host}:${global.database.port}

[libs]
-> .hl-download => https://example.org/x.so
-> key.with.dots => works
]]

local function parse(text, opts)
  return assert(hk.parse(text or SAMPLE, opts))
end

-- ─── module surface ────────────────────────────────────────────────────────

test("module exposes version and entry points", function()
  truthy(type(hk.VERSION) == "string" and hk.VERSION:match("^%d+%.%d+"), "VERSION")
  for _, f in ipairs({ "parse", "parse_file", "load", "new", "from_table", "decode", "decode_file", "encode", "encode_file", "is_config" }) do
    eq(type(hk[f]), "function", f)
  end
  eq(hk.load, hk.parse_file, "load is an alias of parse_file")
end)

test("is_config", function()
  truthy(hk.is_config(parse()))
  truthy(not hk.is_config({}))
  truthy(not hk.is_config(nil))
end)

-- ─── reading values ────────────────────────────────────────────────────────

test("typed getters read every scalar type", function()
  local c = parse()
  eq(c:get_string("global.name"), "Hacker Lang Parser Demo")
  eq(c:get_number("global.version"), 2)
  eq(c:get_number("global.pi"), 3.14159)
  eq(c:get_bool("global.production"), true)
  eq(c:get_string("global.database.host"), "localhost")
  eq(c:get_string("global.database.credentials.username"), "admin")
end)

test("integral numbers come back as integers on 5.3+", function()
  local c = parse()
  if math.type then
    eq(math.type(c:get("global.port")), "integer")
    eq(math.type(c:get("global.pi")), "float")
  end
  eq(c:get("global.port"), 8080)
end)

test("get returns plain tables for arrays and maps", function()
  local c = parse()
  eq(c:get("global.tags"), { "rust", "parser", "hk-format" })
  eq(c:get("arrays.mixed"), { 42, "text", true, 3.14 })
  eq(c:get("global.database.credentials"), { username = "admin" })
  eq(c:get("arrays.multi"), { "a", "b" })
end)

test("array element paths are 0-based, like the file format", function()
  local c = parse()
  eq(c:get("global.tags[0]"), "rust")
  eq(c:get("global.tags[2]"), "hk-format")
  eq(c:get("global.tags[3]"), nil)
end)

test("missing keys: nil or the supplied default", function()
  local c = parse()
  eq(c:get("nope.nothing"), nil)
  eq(c:get("nope.nothing", "dflt"), "dflt")
  eq(c:get("nope", false), false, "false is a legitimate default")
  eq(c:get_string("nope", "d"), "d")
  eq(c:get_number("nope", 7), 7)
  eq(c:get_bool("nope", true), true)
  eq(c:get_number("nope"), nil)
  truthy(not c:has("nope.nothing"))
  truthy(c:has("global.name"))
end)

test("type_of vocabulary", function()
  local c = parse()
  eq(c:type_of("global.name"), "string")
  eq(c:type_of("global.port"), "number")
  eq(c:type_of("global.production"), "bool")
  eq(c:type_of("global.tags"), "array")
  eq(c:type_of("global.database"), "map")
  eq(c:type_of("global"), "map")
  eq(c:type_of("nope"), nil)
end)

test("get_string renders numbers and booleans as text", function()
  local c = parse()
  eq(c:get_string("global.port"), "8080")
  eq(c:get_string("global.production"), "true")
end)

test("typed getters raise a string error on a type mismatch", function()
  local c = parse()
  -- (no `return f()` here: a tail call would erase the caller's frame and with
  -- it the "file:line:" prefix we want to check)
  local e = raises(function() local v = c:get_number("global.name"); return v end, "global.name")
  truthy(e:find("run.lua", 1, true), "error should point at the caller, got: " .. e)
  raises(function() return c:get_bool("global.port") end, "global.port")
  raises(function() return c:get_string("global.tags") end, "global.tags")
end)

test("expect raises on a missing field", function()
  local c = parse()
  eq(c:expect("global.name"), "Hacker Lang Parser Demo")
  raises(function() c:expect("global.nope") end, "missing field: global.nope")
end)

test("bad path argument is a string error", function()
  local c = parse()
  raises(function() c:get(42) end, "path must be a string")
  raises(function() c:get() end, "path must be a string")
end)

test("dotted keys in a file become nested maps", function()
  -- parser behaviour (not the bindings'): `-> key.with.dots => v` nests.
  local c = parse()
  eq(c:get_string("libs.key.with.dots"), "works")
  eq(c:type_of("libs.key"), "map")
end)

test("list paths address keys that dotted paths can't", function()
  local c = parse()
  eq(c:get({ "libs", ".hl-download" }), "https://example.org/x.so")
  eq(c:get_string({ "libs", "key", "with", "dots" }), "works")
  eq(c:get({ "global", "tags" }), { "rust", "parser", "hk-format" })
  truthy(c:has({ "libs", ".hl-download" }))
  truthy(not c:has({ "libs", "nope" }))
end)

-- ─── structure & order ─────────────────────────────────────────────────────

test("sections/keys/len follow file order", function()
  local c = parse()
  eq(c:sections(), { "global", "arrays", "refs", "libs" })
  eq(c:keys(), { "global", "arrays", "refs", "libs" })
  eq(c:keys("global"), { "name", "version", "port", "pi", "production", "tags", "database" })
  eq(c:keys("global.database"), { "host", "port", "credentials" })
  eq(c:keys("global.name"), nil)
  eq(c:len(), 4)
  eq(c:len("global.tags"), 3)
  eq(c:len("global.database"), 3)
  eq(c:len("global.name"), nil)
  eq(c:len("nope"), nil)
end)

test("each iterates maps in file order and arrays by index", function()
  local c = parse()
  local seen = {}
  for k, v in c:each("global.database") do seen[#seen + 1] = k end
  eq(seen, { "host", "port", "credentials" })

  local sections = {}
  for name in c:each() do sections[#sections + 1] = name end
  eq(sections, { "global", "arrays", "refs", "libs" })

  local items = {}
  for i, v in c:each("global.tags") do items[#items + 1] = i .. "=" .. v end
  eq(items, { "1=rust", "2=parser", "3=hk-format" })

  local n = 0
  for _ in c:each("nope") do n = n + 1 end
  eq(n, 0)
  for _ in c:each("global.name") do n = n + 1 end
  eq(n, 0)
end)

test("each works with keys that start with or contain dots", function()
  local c = parse()
  local got = {}
  for k, v in c:each("libs") do got[k] = v end
  eq(got[".hl-download"], "https://example.org/x.so")
  eq(got.key, { with = { dots = "works" } })
  c:set({ "libs", "x.y" }, "literal")
  local lit = {}
  for k, v in c:each("libs") do lit[k] = v end
  eq(lit["x.y"], "literal")
end)

test("to_table mirrors the document", function()
  local t = parse():to_table()
  eq(t.global.port, 8080)
  eq(t.global.database.credentials.username, "admin")
  eq(t.arrays.numbers, { 1, 2, 3 })
end)

-- ─── interpolation ─────────────────────────────────────────────────────────

test("resolve option expands references and env vars", function()
  local c = parse(SAMPLE, { resolve = true })
  eq(c:get_string("refs.from_global"), "Hacker Lang Parser Demo")
  eq(c:get_string("refs.composed"), "localhost:5432")
  eq(c:get_string("refs.home"), os.getenv("PATH") or "")
  eq(c:get_string("refs.first_number"), "1")
end)

test("unresolved text keeps the raw ${...}", function()
  eq(parse():get_string("refs.from_global"), "${global.name}")
end)

test("resolve() works in place and is chainable", function()
  local c = parse()
  local r = c:resolve()
  truthy(r == c, "resolve returns self")
  eq(c:get_string("refs.composed"), "localhost:5432")
end)

test("cyclic reference is a data error, not a raise", function()
  local c, err, info = hk.parse("[a]\n-> x => ${a.y}\n-> y => ${a.x}\n", { resolve = true })
  eq(c, nil)
  truthy(err:find("Cyclic", 1, true), err)
  eq(info.kind, "cycle")

  local c2 = assert(hk.parse("[a]\n-> x => ${a.y}\n-> y => ${a.x}\n"))
  local r, err2, info2 = c2:resolve()
  eq(r, nil)
  eq(info2.kind, "cycle")
  truthy(err2 ~= nil)
end)

test("invalid reference", function()
  local c, err, info = hk.parse("[a]\n-> x => ${nope.nothing}\n", { resolve = true })
  eq(c, nil)
  eq(info.kind, "reference")
end)

-- ─── errors ────────────────────────────────────────────────────────────────

test("parse errors carry line, column and a plain-text snippet", function()
  local c, err, info = hk.parse("[ok]\n-> a => 1\n-> broken\nnot a key line\n")
  eq(c, nil)
  eq(type(err), "string")
  eq(info.kind, "parse")
  eq(type(info.line), "number")
  eq(type(info.column), "number")
  truthy(info.line >= 3, "line " .. tostring(info.line))
  truthy(type(info.rendered) == "string" and #info.rendered > 0, "rendered")
  truthy(not info.rendered:find("\27", 1, true), "rendered output must not contain ANSI escapes")
  truthy(info.rendered:find("^", 1, true), "caret")
end)

test("parse error: text before any section header", function()
  local _, _, info = hk.parse("oops")
  eq(info.kind, "parse")
  eq(info.line, 1)
  eq(info.column, 1)
  eq(info.detail, "Expected section header")
end)

test("missing file is an io error", function()
  local c, err, info = hk.load("/definitely/not/here.hk")
  eq(c, nil)
  eq(info.kind, "io")
  truthy(type(err) == "string")
end)

test("argument type errors raise strings", function()
  raises(function() hk.parse(42) end, "string expected")
  raises(function() hk.parse("[a]", 5) end, "options table")
  raises(function() hk.load(nil) end, "file path")
  raises(function() hk.from_table("x") end, "table expected")
end)

-- ─── mutation ──────────────────────────────────────────────────────────────

test("set overwrites in place, keeping key order", function()
  local c = parse()
  truthy(c:set("global.name", "renamed") == c, "set returns self")
  eq(c:get_string("global.name"), "renamed")
  eq(c:keys("global")[1], "name")
end)

test("set creates sections and maps, appends new keys last", function()
  local c = parse()
  c:set("build.opt.level", 3):set("global.extra", true)
  eq(c:get_number("build.opt.level"), 3)
  eq(c:sections()[#c:sections()], "build")
  local gk = c:keys("global")
  eq(gk[#gk], "extra")
end)

test("set accepts tables (arrays and maps) and list paths", function()
  local c = parse()
  c:set("build.targets", { "a", "b" })
  c:set("build.meta", { owner = "me", n = 2 })
  c:set({ "libs", "x.y" }, "dotted")
  eq(c:get("build.targets"), { "a", "b" })
  eq(c:get("build.meta"), { owner = "me", n = 2 })
  eq(c:get({ "libs", "x.y" }), "dotted")
end)

test("array elements: replace, append, reject gaps", function()
  local c = parse()
  c:set("global.tags[0]", "RUST")
  c:set("global.tags[3]", "new")
  eq(c:get("global.tags"), { "RUST", "parser", "hk-format", "new" })
  raises(function() c:set("global.tags[10]", "gap") end, "global.tags[10]")
end)

test("set raises on impossible targets", function()
  local c = parse()
  raises(function() c:set("loose", 1) end, "section")
  raises(function() c:set("global.name.sub", 1) end, "global.name.sub")
  raises(function() c:set("global.x", function() end) end, "function")
  raises(function() c:set("global.x", 0 / 0) end, "NaN")
end)

test("set(path, nil) and delete remove values", function()
  local c = parse()
  c:set("global.production", nil)
  truthy(not c:has("global.production"))
  eq(c:delete("global.tags[0]"), true)
  eq(c:get("global.tags"), { "parser", "hk-format" })
  eq(c:delete("global.database"), true)
  eq(c:delete("global.database"), false)
  eq(c:keys("global"), { "name", "version", "port", "pi", "tags" })
end)

test("clone is independent", function()
  local a = parse()
  local b = a:clone()
  b:set("global.name", "changed")
  eq(a:get_string("global.name"), "Hacker Lang Parser Demo")
  eq(b:get_string("global.name"), "changed")
end)

-- ─── serialisation / files ─────────────────────────────────────────────────

test("serialize round-trips", function()
  local a = parse()
  local b = assert(hk.parse(a:serialize()))
  eq(b:to_table(), a:to_table())
  eq(tostring(a), a:serialize())
end)

test("write_file / load round-trip", function()
  local path = os.tmpname()
  local a = parse()
  a:set("extra.note", "héllo wörld ✓")
  assert(a:write_file(path))
  local b = assert(hk.load(path))
  os.remove(path)
  eq(b:get_string("extra.note"), "héllo wörld ✓")
  eq(b:to_table(), a:to_table())
end)

test("write_file to an unwritable location is an io error", function()
  local ok, err, info = parse():write_file("/definitely/not/here/x.hk")
  eq(ok, nil)
  eq(info.kind, "io")
  truthy(type(err) == "string")
end)

test("load with resolve option", function()
  local path = os.tmpname()
  local f = assert(io.open(path, "w"))
  f:write("[a]\n-> x => 1\n-> y => v${a.x}\n")
  f:close()
  local c = assert(hk.load(path, { resolve = true }))
  os.remove(path)
  eq(c:get_string("a.y"), "v1")
end)

test("unicode and escapes survive", function()
  local c = assert(hk.parse('[u]\n-> s => "zażółć gęślą jaźń"\n-> q => "say \\"hi\\""\n'))
  eq(c:get_string("u.s"), "zażółć gęślą jaźń")
  local back = assert(hk.parse(c:serialize()))
  eq(back:get_string("u.q"), c:get_string("u.q"))
end)

-- ─── tables in / out ───────────────────────────────────────────────────────

test("from_table / encode / decode", function()
  local src = {
    package = { name = "demo", version = 1.5, tags = { "a", "b c" }, nested = { on = true } },
    empty = {},
  }
  local c = assert(hk.from_table(src))
  eq(c:sections(), { "empty", "package" }, "sections are alphabetical")
  eq(c:keys("package"), { "name", "nested", "tags", "version" }, "keys are alphabetical")
  local text = assert(hk.encode(src))
  local back = assert(hk.decode(text))
  eq(back.package.name, "demo")
  eq(back.package.version, 1.5)
  eq(back.package.tags, { "a", "b c" })
  eq(back.package.nested.on, true)
end)

test("from_table rejects unrepresentable input with nil, msg, info", function()
  local c, err, info = hk.from_table({ a = 1 })
  eq(c, nil)
  eq(info.kind, "type")
  truthy(err:find("section 'a'", 1, true), err)

  local _, e2 = hk.from_table({ a = { 1, 2, 3 } })
  truthy(e2:find("must be a map", 1, true), e2)

  local _, e3 = hk.from_table({ a = { [1] = "x", y = "z" } })
  truthy(e3:find("keys must be strings", 1, true), e3)

  local _, e4 = hk.from_table({ a = { f = print } })
  truthy(e4:find("function", 1, true), e4)

  local cyc = { s = {} }
  cyc.s.self = cyc.s
  local _, e5 = hk.from_table(cyc)
  truthy(e5:find("deeper", 1, true), e5)
end)

test("decode_file / encode_file", function()
  local path = os.tmpname()
  assert(hk.encode_file({ s = { k = "v", n = 3 } }, path))
  local t = assert(hk.decode_file(path))
  os.remove(path)
  eq(t, { s = { k = "v", n = 3 } })
end)

test("new() starts empty", function()
  local c = hk.new()
  eq(c:len(), 0)
  eq(c:sections(), {})
  c:set("a.b", 1)
  eq(c:get("a.b"), 1)
end)

test("__len and pairs (where the Lua version supports them)", function()
  local c = parse()
  if _VERSION ~= "Lua 5.1" then eq(#c, 4) end
  if _VERSION ~= "Lua 5.1" or rawget(_G, "jit") then
    local names = {}
    for name in pairs(c) do names[#names + 1] = name end
    -- plain-5.1 has no __pairs: skip the assertion there (jit builds have it)
    if #names > 0 and names[1] ~= "_c" then eq(names, { "global", "arrays", "refs", "libs" }) end
  end
end)

test("a real .hk file from the repository", function()
  local path = (os.getenv("HK_PARSER_EXAMPLE") or "examples/config.hk")
  local f = io.open(path, "r")
  if not f then return end -- run from another directory: nothing to check
  f:close()
  local c = assert(hk.load(path))
  eq(c:get_string("global.name"), "Hacker Lang Parser Demo")
  eq(c:get_number("global.database.port"), 5432)
  eq(c:get("deep.a.b.c"), 42)
  eq(c:get_string("deep.x.y.z"), "nested value")
end)

-- ─── report ────────────────────────────────────────────────────────────────

print(("\n%d passed, %d failed  (%s, hk-parser %s)"):format(passed, failed, _VERSION, hk.VERSION))
for _, f in ipairs(failures) do
  print("\nFAIL: " .. f.name .. "\n" .. f.err)
end
os.exit(failed == 0 and 0 or 1)
