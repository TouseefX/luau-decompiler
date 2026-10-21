local function make_counter(start)
    local n = start
    return function(step)
        n = n + step
        return n
    end, function() return n end
end
local inc, get = make_counter(10)
print(inc(5), inc(2), get())
local fns = {}
for i = 1, 5 do
    fns[i] = function() return i * i end
end
local sum = 0
for i = 1, 5 do sum = sum + fns[i]() end
print(sum)
local function outer(a)
    local b = a * 2
    return function(c)
        return function(d)
            return a + b + c + d
        end
    end
end
print(outer(1)(2)(3))
local shared = 0
local function a1() shared = shared + 1; return shared end
local function a2() shared = shared + 10; return shared end
print(a1(), a2(), a1(), shared)
