local s = "Hello, World"
print(#s, s:upper(), s:lower(), s:rep(2), s:sub(1, 5), s:sub(-5))
print(s:find("World"), s:find("xyz"))
print(s:gsub("o", "0"), s:gsub("l", "L", 2))
print(string.format("%d %5.2f %s %q", 42, 3.14159, "hi", "a\nb"))
print(("a,b,c"):split(",")[2])
local parts = {}
for w in string.gmatch("one two three", "%a+") do
    table.insert(parts, w)
end
print(table.concat(parts, "-"))
print(string.byte("A"), string.char(72, 105))
print(("test"):reverse())
print(("  trim  "):match("^%s*(.-)%s*$"))
local interp = `sum: {1 + 2}, nested: {("x"):upper()}`
print(interp)
print(string.format("%x %X %o %c", 255, 255, 8, 65))
print(tostring(1/3), tostring(1e300 * 1e10))
