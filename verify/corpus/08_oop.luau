local Animal = {}
Animal.__index = Animal
function Animal.new(name, sound)
    local self = setmetatable({}, Animal)
    self.name = name
    self.sound = sound
    return self
end
function Animal:speak() return self.name .. " says " .. self.sound end
function Animal:getName() return self.name end
local Dog = setmetatable({}, {__index = Animal})
Dog.__index = Dog
function Dog.new(name)
    local self = Animal.new(name, "woof")
    return setmetatable(self, Dog)
end
function Dog:fetch() return self:getName() .. " fetches!" end
local d = Dog.new("Rex")
print(d:speak(), d:fetch())
local Stack = {}
Stack.__index = Stack
function Stack.new() return setmetatable({items = {}, size = 0}, Stack) end
function Stack:push(v) self.size = self.size + 1; self.items[self.size] = v end
function Stack:pop()
    if self.size == 0 then return nil end
    local v = self.items[self.size]
    self.items[self.size] = nil
    self.size = self.size - 1
    return v
end
local st = Stack.new()
st:push("a") st:push("b") st:push("c")
print(st:pop(), st:pop(), st.size, st:pop(), st:pop())
local function chain(s) return s:upper():lower():rep(2) end
print(chain("Ab"))
