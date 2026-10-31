local core = require("hk_parser_core")

local type, error, setmetatable, getmetatable = type, error, setmetatable, getmetatable

local M = {
  --- The raw native module (every function returns `nil, msg, info` on failure
  --- and never raises on bad data). Rarely needed.
  core = core,
  --- Version of the underlying Rust crate.
  VERSION = core.version,
}

---@class HkConfig
local Config = {}
Config.__index = Config
Config.__name = "hk_parser.Config"

local function wrap(native)
  return setmetatable({ _c = native }, Config)
end

-- ─── argument checking ─────────────────────────────────────────────────────
-- Helpers are called from API functions, so level 3 = the user's call site.

local function check_path(path)
  local t = type(path)
  if t ~= "string" and t ~= "table" then
    error('bad argument #1 (path must be a string like "a.b[0]" or a list of keys, got ' .. t .. ")", 3)
  end
end

local function check_opt_path(path)
  if path ~= nil then
    check_path(path)
  end
end

local function check_text(v, what)
  if type(v) ~= "string" then
    error("bad argument #1 (" .. what .. " expected, got " .. type(v) .. ")", 3)
  end
end

local function check_opts(opts)
  if opts ~= nil and type(opts) ~= "table" then
    error("bad argument #2 (options table expected, got " .. type(opts) .. ")", 3)
  end
end

-- ─── constructors ──────────────────────────────────────────────────────────

--- Parse `.hk` source text.
---@param text string
---@param opts? { resolve?: boolean }  `resolve = true` expands `${...}` right away
---@return HkConfig|nil cfg, string? err, table? info
function M.parse(text, opts)
  check_text(text, "string")
  check_opts(opts)
  local c, err, info = core.parse(text, opts)
  if c == nil then
    return nil, err, info
  end
  return wrap(c)
end

--- Parse a `.hk` file from disk.
---@param path string
---@param opts? { resolve?: boolean }
---@return HkConfig|nil cfg, string? err, table? info
function M.parse_file(path, opts)
  check_text(path, "file path")
  check_opts(opts)
  local c, err, info = core.parse_file(path, opts)
  if c == nil then
    return nil, err, info
  end
  return wrap(c)
end
M.load = M.parse_file

--- An empty document.
---@return HkConfig
function M.new()
  return wrap(core.new())
end

--- Build a document from a plain table `{ section = { key = value, ... } }`.
--- Maps are emitted in alphabetical key order (Lua tables are unordered); use
--- `new()` + `set()` when order matters. A table with keys 1..n is an array.
---@param tbl table
---@return HkConfig|nil cfg, string? err, table? info
function M.from_table(tbl)
  if type(tbl) ~= "table" then
    error("bad argument #1 (table expected, got " .. type(tbl) .. ")", 2)
  end
  local c, err, info = core.from_table(tbl)
  if c == nil then
    return nil, err, info
  end
  return wrap(c)
end

--- `true` if `x` is a document created by this module.
---@param x any
---@return boolean
function M.is_config(x)
  return getmetatable(x) == Config
end

-- ─── plain-table convenience (decode/encode, like cjson) ───────────────────

--- Parse text straight into a plain Lua table.
---@param text string
---@param opts? { resolve?: boolean }
---@return table|nil tbl, string? err, table? info
function M.decode(text, opts)
  local cfg, err, info = M.parse(text, opts)
  if cfg == nil then
    return nil, err, info
  end
  return cfg:to_table()
end

--- Parse a file straight into a plain Lua table.
---@param path string
---@param opts? { resolve?: boolean }
---@return table|nil tbl, string? err, table? info
function M.decode_file(path, opts)
  local cfg, err, info = M.parse_file(path, opts)
  if cfg == nil then
    return nil, err, info
  end
  return cfg:to_table()
end

--- Serialise a plain table (see `from_table`) to `.hk` text.
---@param tbl table
---@return string|nil text, string? err, table? info
function M.encode(tbl)
  local cfg, err, info = M.from_table(tbl)
  if cfg == nil then
    return nil, err, info
  end
  return cfg:serialize()
end

--- Serialise a plain table to a `.hk` file.
---@param tbl table
---@param path string
---@return boolean|nil ok, string? err, table? info
function M.encode_file(tbl, path)
  check_text(path, "file path")
  local cfg, err, info = M.from_table(tbl)
  if cfg == nil then
    return nil, err, info
  end
  return cfg:write_file(path)
end

-- ─── Config: queries ───────────────────────────────────────────────────────

--- Value at `path` as a plain Lua value (maps/arrays become tables), or
--- `default` (nil if omitted) when the path doesn't exist.
function Config:get(path, default)
  check_path(path)
  return self._c:get(path, default)
end

--- Like `get` but raises `"missing field: <path>"` when absent.
function Config:expect(path)
  check_path(path)
  local v = self._c:get(path)
  if v == nil then
    error("missing field: " .. (type(path) == "table" and table.concat(path, ".") or path), 2)
  end
  return v
end

---@return boolean
function Config:has(path)
  check_path(path)
  return self._c:has(path)
end

--- `"string" | "number" | "bool" | "array" | "map"`, or nil if absent.
---@return string|nil
function Config:type_of(path)
  check_path(path)
  return self._c:type_of(path)
end

-- A typed getter returns the value, `default`/nil when the key is absent, and
-- raises when the key exists with another type (e.g. get_number on a string).
local function typed(name)
  return function(self, path, default)
    check_path(path)
    local v, err = self._c[name](self._c, path, default)
    if v == nil and err ~= nil then
      error(err, 2)
    end
    return v
  end
end

--- String value (numbers and booleans are rendered as text).
---@param path string|string[]
---@param default? string
---@return string|nil
Config.get_string = typed("get_string")

--- Number value. Integral numbers come back as integers on Lua 5.3+.
---@param path string|string[]
---@param default? number
---@return number|nil
Config.get_number = typed("get_number")

--- Boolean value.
---@param path string|string[]
---@param default? boolean
---@return boolean|nil
Config.get_bool = typed("get_bool")

--- Element count of the array at `path`, or key count of the map at `path`
--- (no path = number of sections). nil if `path` is neither.
---@return integer|nil
function Config:len(path)
  check_opt_path(path)
  return self._c:len(path)
end

--- Keys of the map at `path` in file order (no path = section names), or nil.
---@return string[]|nil
function Config:keys(path)
  check_opt_path(path)
  return self._c:keys(path)
end

--- Section names in file order.
---@return string[]
function Config:sections()
  return self._c:keys()
end

--- Iterate a map in file order (`for key, value in cfg:each("section")`) or an
--- array (`for index, value in ...`, 1-based). No path iterates the sections.
--- Nested maps/arrays come back as plain tables.
function Config:each(path)
  check_opt_path(path)
  local keys = self._c:keys(path)
  if keys ~= nil then
    local tbl = (path == nil or path == "" or (type(path) == "table" and #path == 0))
      and self._c:to_table() or self._c:get(path)
    local i = 0
    return function()
      i = i + 1
      local k = keys[i]
      if k ~= nil then
        return k, tbl[k]
      end
    end
  end
  local arr = self._c:get(path)
  if type(arr) ~= "table" or self._c:type_of(path) ~= "array" then
    return function() end
  end
  local i = 0
  return function()
    i = i + 1
    if i <= #arr then
      return i, arr[i]
    end
  end
end

--- The whole document as nested plain tables (map order is not preserved —
--- use `keys()`/`each()` when order matters).
---@return table
function Config:to_table()
  return self._c:to_table()
end

-- ─── Config: mutation & output ─────────────────────────────────────────────

--- Expand `${section.key}` / `${env:NAME}` in place.
--- Returns `self`, or `nil, err, info` on a cyclic/invalid reference.
function Config:resolve()
  local r, err, info = self._c:resolve()
  if r == nil then
    return nil, err, info
  end
  return self
end

--- Set `path` to `value` (nil deletes). Missing sections/maps are created;
--- an existing key keeps its position. `"list[i]"` replaces element `i` or
--- appends when `i == #list`. Returns `self` for chaining; raises if the path
--- can't hold the value (e.g. walking through a string).
function Config:set(path, value)
  check_path(path)
  local r, err = self._c:set(path, value)
  if r == nil then
    error(err, 2)
  end
  return self
end

--- Remove the value at `path`. Returns whether anything was removed.
---@return boolean
function Config:delete(path)
  check_path(path)
  return self._c:delete(path)
end

--- Independent deep copy.
---@return HkConfig
function Config:clone()
  return wrap(self._c:clone())
end

--- `.hk` text for the whole document.
---@return string
function Config:serialize()
  return self._c:serialize()
end

--- Write the document to `path`. Returns `true` or `nil, err, info`.
---@return boolean|nil ok, string? err, table? info
function Config:write_file(path)
  check_text(path, "file path")
  return self._c:write_file(path)
end

Config.__tostring = Config.serialize

Config.__len = function(self)
  return self._c:len()
end

-- `pairs(cfg)` iterates sections in file order (Lua 5.2+/LuaJIT-5.2-compat;
-- on plain 5.1 use `cfg:each()`).
Config.__pairs = function(self)
  return self:each(), nil, nil
end

return M
