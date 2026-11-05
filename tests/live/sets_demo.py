numbers = {3, 1, 2, 2, 3}
print("literal:", numbers, "len:", len(numbers), "in:", 2 in numbers)
assert numbers == {1, 2, 3}
assert sorted(numbers) == [1, 2, 3]

numbers.add(4)
numbers.remove(1)
numbers.discard(99)
print("add/remove:", sorted(numbers))
assert sorted(numbers) == [2, 3, 4]

left = {1, 2, 3}
right = {3, 4}
print("union:", sorted(left.union(right)))
print("intersection:", sorted(left.intersection(right)))
print("difference:", sorted(left.difference(right)))
assert sorted(left.union(right)) == [1, 2, 3, 4]
assert sorted(left.intersection(right)) == [3]
assert sorted(left.difference(right)) == [1, 2]
assert left.issubset({1, 2, 3, 4})
assert left.issuperset({1, 2})
assert not left.issubset({1, 2})

constructed = set([1, 2, 2, 3])
print("constructor:", constructed)
assert constructed == {1, 2, 3}

squares = {value * value for value in range(6) if value % 2 == 0}
print("set comp:", sorted(squares))
assert sorted(squares) == [0, 4, 16]

mapping = {str(value): value * 10 for value in range(3)}
print("dict comp:", mapping)
assert mapping["2"] == 20

for value in sorted({5, 1, 3}):
    print("iter", value)

try:
    {1, 2}.remove(9)
    missing = False
except KeyError:
    missing = True
assert missing
print("sets ok")
