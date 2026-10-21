local function count(...)
    return select("#", ...)
end
local function sum(...)
    local total = 0
    for i = 1, select("#", ...) do
        total = total + (select(i, ...))
    end
    return total
end
print(count(), count(1), count(nil, nil), count(1, 2, 3))
print(sum(1, 2, 3, 4, 5))
local function fwd(...)
    return ...
end
print(fwd("a", "b", "c"))
local packed = table.pack(1, nil, 3)
print(packed.n, packed[1], packed[3], packed[2] == nil)
print(table.unpack({1, 2, 3}))
local function mid(...)
    return 0, ...
end
print(mid(7, 8, 9))
print((fwd(1, 2, 3)))
local function tail(...)
    return count(...), fwd(...)
end
print(tail("x", "y"))
