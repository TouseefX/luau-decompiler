local Vec = {}
Vec.__index = Vec
function Vec.new(x, y)
    return setmetatable({x = x, y = y}, Vec)
end
function Vec:add(o)
    return Vec.new(self.x + o.x, self.y + o.y)
end
function Vec:len()
    return math.sqrt(self.x * self.x + self.y * self.y)
end
Vec.__add = Vec.add
Vec.__tostring = function(v) return "(" .. v.x .. ", " .. v.y .. ")" end
Vec.__eq = function(a, b) return a.x == b.x and a.y == b.y end
local a, b = Vec.new(1, 2), Vec.new(3, 4)
print(tostring(a + b), a == Vec.new(1, 2), a == b)
print(string.format("%.2f", a:len()))
local proxy = setmetatable({}, {
    __index = function(t, k) return k .. "!" end,
    __newindex = function(t, k, v) rawset(t, k, v .. "?") end,
    __len = function() return 42 end,
    __call = function(self, x) return "called " .. x end,
})
print(proxy.foo)
proxy.bar = "baz"
print(rawget(proxy, "bar"), #proxy, proxy("hey"))
local Base = {greet = function() return "base" end}
local Mid = setmetatable({extra = function() return "mid" end}, {__index = Base})
local Leaf = setmetatable({}, {__index = Mid})
print(Leaf.greet(), Leaf.extra())
local co = setmetatable({}, {__concat = function(x, y) return "concat!" end})
print(co .. 5, "s" .. co)
