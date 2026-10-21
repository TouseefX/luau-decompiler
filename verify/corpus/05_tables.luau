local t = {1, 2, 3, x = 10, ["key with space"] = true, [5] = "five"}
print(#t, t[1], t.x, t["key with space"], t[5])
local nested = {a = {b = {c = {d = "deep"}}}}
print(nested.a.b.c.d)
local arr = {}
for i = 1, 100 do arr[i] = i * 2 end
print(#arr, arr[50], arr[100])
table.insert(arr, 1, 0)
table.insert(arr, 999)
print(arr[1], arr[2], #arr, arr[#arr])
print(table.remove(arr), table.remove(arr, 1), #arr)
print(table.concat({1, 2, 3, 4, 5}, "+"))
local sorted = {5, 2, 8, 1, 9, 3}
table.sort(sorted)
print(table.concat(sorted, ","))
table.sort(sorted, function(a, b) return a > b end)
print(table.concat(sorted, ","))
local words = {"banana", "apple", "Cherry"}
table.sort(words, function(a, b) return a:lower() < b:lower() end)
print(table.concat(words, " "))
local created = table.create(3, "x")
print(#created, created[1], created[3])
local frozen = table.freeze({a = 1})
print(frozen.a, table.isfrozen(frozen))
print(table.find({10, 20, 30}, 20))
local clone = table.clone(t)
clone[1] = 99
print(t[1], clone[1])
