local Module = {}
Module.__index = Module

local DEFAULTS = {retries = 3, timeout = 100, verbose = false}

local function merge(dst, src)
    for k, v in pairs(src) do
        if type(v) == "table" and type(dst[k]) == "table" then
            merge(dst[k], v)
        else
            dst[k] = v
        end
    end
    return dst
end

function Module.new(opts)
    local cfg = merge({}, DEFAULTS)
    if opts then merge(cfg, opts) end
    return setmetatable({config = cfg, log = {}, count = 0}, Module)
end

function Module:record(msg)
    self.count = self.count + 1
    table.insert(self.log, string.format("[%03d] %s", self.count, msg))
end

function Module:run(items)
    local results = {}
    for idx, item in ipairs(items) do
        self:record("processing " .. tostring(item))
        local ok, err = pcall(function()
            if type(item) ~= "number" then error("bad item: " .. tostring(item)) end
            return item * 2
        end)
        results[idx] = ok and err or ("E:" .. tostring(err))
    end
    return results
end

function Module:summary()
    local parts = {}
    for _, line in ipairs(self.log) do table.insert(parts, line) end
    return table.concat(parts, "\n")
end

local m = Module.new({verbose = true, extra = {nested = {deep = 1}}})
local res = m:run({1, 2, "x", 4})
print(table.concat(res, ","))
print(m.count, m.config.retries, m.config.extra.nested.deep)
print(m:summary())
