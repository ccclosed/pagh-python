items = [3, 1, 2]
items.sort()
items.append(4)
items.extend([5, 6])
print("list:", items, "pop:", items.pop(0), "slice:", items[1:4], "reversed:", items[::-1])

counts = {"apple": 2, "pear": 1}
counts["plum"] = 3
print("dict:", counts, "keys:", sorted(counts.keys()))
print("get:", counts.get("missing", 0), "items:", counts.items())

text = "  Hello, Python World  "
print("str:", text.strip().lower())
print("split:", "a,b,,c".split(","), "join:", "-".join(["x", "y", "z"]))
print("find:", "banana".find("na"), "count:", "banana".count("na"))
print("replace:", "banana".replace("a", "o", 2), "title:", "hello world".title())
print("starts/ends:", "report.txt".startswith("rep"), "report.txt".endswith(".txt"))
print("slices:", "abcdef"[1:5], "abcdef"[::2], text.strip()[0:5])

for index, value in enumerate(["a", "b"]):
    print("enumerate", index, value)
for left, right in zip([1, 2], [3, 4]):
    print("zip", left, right)
print("min/max/sum:", min(items), max(items), sum(items))
print("sorted:", sorted([5, 2, 9, 1]), sorted(["b", "a"], reverse=True))
