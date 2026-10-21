local function word_count(s)
    local counts = {}
    for w in s:gmatch("%w+") do
        counts[w:lower()] = (counts[w:lower()] or 0) + 1
    end
    local keys = {}
    for k in pairs(counts) do table.insert(keys, k) end
    table.sort(keys)
    local out = {}
    for _, k in ipairs(keys) do table.insert(out, k .. "=" .. counts[k]) end
    return table.concat(out, " ")
end
print(word_count("the quick brown fox jumps over the lazy dog the end"))
local function caesar(s, shift)
    local out = {}
    for i = 1, #s do
        local c = s:sub(i, i)
        local b = c:byte()
        if b >= 65 and b <= 90 then
            out[i] = string.char((b - 65 + shift) % 26 + 65)
        elseif b >= 97 and b <= 122 then
            out[i] = string.char((b - 97 + shift) % 26 + 97)
        else
            out[i] = c
        end
    end
    return table.concat(out)
end
print(caesar("Hello, World!", 3))
print(caesar(caesar("Hello, World!", 3), -3))
local csv = 'a,"b,c",d'
local fields = {}
for field in (csv .. ","):gmatch('("?)(.-)%1,') do
    if field ~= "" then table.insert(fields, field) end
end
print(#fields)
local function pad(s, n, fill)
    fill = fill or " "
    s = tostring(s)
    local need = n - #s
    if need <= 0 then return s end
    return fill:rep(need):sub(1, need) .. s
end
print("[" .. pad("7", 5) .. "]", "[" .. pad("abc", 6, ".") .. "]")
