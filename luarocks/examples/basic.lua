local hk = require("hk_parser")

local path = arg and arg[1] or "examples/config.hk"

-- 1. Load + resolve ${...} / ${env:...}. Data errors come back as nil, msg, info.
local cfg, err, info = hk.load(path, { resolve = true })
if not cfg then
  io.stderr:write(info.rendered or err, "\n")   -- rustc-style snippet for parse errors
  os.exit(1)
end

-- 2. Typed reads with defaults (paths: "section.key", "list[0]").
print("name    :", cfg:get_string("global.name"))
print("version :", cfg:get_number("global.version"))
print("db port :", cfg:get_number("global.database.port", 5432))
print("ssl     :", cfg:get_bool("global.database.ssl", false))
print("first tag:", cfg:get("global.tags[0]"))

-- 3. Walk a section in file order.
print("\n[global.database]")
for key, value in cfg:each("global.database") do
  print("  " .. key, type(value) == "table" and "<map>" or tostring(value))
end

-- 4. Edit and serialise; key order is preserved, new keys are appended.
cfg:set("global.version", 3)
   :set("build.targets", { "linux", "hackeros" })
   :delete("global.database.credentials")
print("\n" .. cfg:serialize())

-- 5. Plain-table API, cjson style.
local t = assert(hk.decode("[app]\n-> debug => true\n-> workers => 4\n"))
print(t.app.debug, t.app.workers)
print(assert(hk.encode({ app = { name = "demo", tags = { "a", "b" } } })))
