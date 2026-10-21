local function primes(limit)
    local sieve = {}
    for i = 2, limit do sieve[i] = true end
    for i = 2, math.floor(math.sqrt(limit)) do
        if sieve[i] then
            for j = i * i, limit, i do sieve[j] = false end
        end
    end
    local out = {}
    for i = 2, limit do if sieve[i] then table.insert(out, i) end end
    return out
end
print(table.concat(primes(50), " "))
local function quicksort(t, lo, hi)
    lo = lo or 1
    hi = hi or #t
    if lo < hi then
        local pivot = t[hi]
        local i = lo - 1
        for j = lo, hi - 1 do
            if t[j] <= pivot then
                i = i + 1
                t[i], t[j] = t[j], t[i]
            end
        end
        t[i + 1], t[hi] = t[hi], t[i + 1]
        local p = i + 1
        quicksort(t, lo, p - 1)
        quicksort(t, p + 1, hi)
    end
    return t
end
print(table.concat(quicksort({9, 3, 7, 1, 8, 2, 6, 4, 5}), ","))
local function matmul(a, b)
    local c = {}
    for i = 1, #a do
        c[i] = {}
        for j = 1, #b[1] do
            local s = 0
            for k = 1, #b do s = s + a[i][k] * b[k][j] end
            c[i][j] = s
        end
    end
    return c
end
local m = matmul({{1, 2}, {3, 4}}, {{5, 6}, {7, 8}})
print(m[1][1], m[1][2], m[2][1], m[2][2])
local memo = {}
local function fib(n)
    if n < 2 then return n end
    if memo[n] then return memo[n] end
    local v = fib(n - 1) + fib(n - 2)
    memo[n] = v
    return v
end
print(fib(60))
local gi = 0
for k, v in pairs({a = 1}) do gi = gi + v end
for idx, val in ipairs({10, 20, 30}) do gi = gi + idx * val end
print(gi)
