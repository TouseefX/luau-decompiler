local out = {}
local i = 0
while i < 10 do
    i = i + 1
    if i % 2 == 0 then continue end
    table.insert(out, i)
end
print(table.concat(out, ","))
local j = 0
repeat
    j = j + 1
until j >= 5
print(j)
for k = 10, 1, -3 do print(k) end
local frag = {}
for k = 1, 2, 0.5 do table.insert(frag, tostring(k)) end
print(table.concat(frag, " "))
local n = 0
for a = 1, 5 do
    for b = 1, 5 do
        if a * b > 6 then break end
        n = n + 1
    end
    if n > 10 then break end
end
print(n)
local x = 5
if x > 3 then print("big") elseif x > 1 then print("mid") else print("small") end
local cond1, cond2 = true, false
if cond1 and not cond2 then print("logic ok") end
local v = nil
local r = v and v.field or "default"
print(r)
print(1 < 2 and 2 < 3 and "chain" or "no")
