print(7 // 2, -7 // 2, 7 % 3, -7 % 3, 2 ^ 10, 7 / 2)
print(math.huge, -math.huge, math.pi)
print(math.type(1), math.type(1.5))
print(0.1 + 0.2, 1e15 + 1, 2^53)
print(0xFF % 16, 2 ^ 3, 6 // 4)
print(2 ^ 10, 1024 / 8, -8 // 2)
local a, b = 1, 2
a, b = b, a
print(a, b)
local t = {1, 2, 3}
t[1], t[3] = t[3], t[1]
print(t[1], t[2], t[3])
local x = 10
do
    local x = 20
    print(x)
    do
        local x = 30
        print(x)
    end
    print(x)
end
print(x)
local function multi() return 1, 2, 3 end
local p, q, r, s = multi()
print(p, q, r, s)
local u, v = multi(), 10
print(u, v)
print(("abc"):byte(1, -1))
print(#"", #"abc", #{1, 2, 3})
print(tostring(true), tostring(nil), tonumber("42"), tonumber("z"), tonumber("0x10"))
print(math.floor(-3.5), math.ceil(-3.5), math.round(2.5), math.abs(-4))
print(math.max(1, 5, 3), math.min(1, 5, 3))
