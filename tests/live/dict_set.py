# Live proof for dict views, bitwise operators and set operations in
# pythonc 0.8.0.
# Run: run /mnt/pythonc /mnt/dict_set.py
#
# Three things are checked here: `d.keys()/values()/items()` are live view
# objects (len/iter/membership, not lists), the bitwise operators `& | ^ ~
# << >>` exist (int semantics, Python precedence), and set algebra is
# available both as operators and as the mutating methods (`|=` updates the
# same object, so an alias sees it).

# ── dict views ────────────────────────────────────────────────────────────

counts = {"a": 1, "b": 2}
keys = counts.keys()
values = counts.values()
items = counts.items()

assert (len(keys), len(values), len(items)) == (2, 2, 2)
assert list(keys) == ["a", "b"]
assert list(values) == [1, 2]
assert list(items) == [("a", 1), ("b", 2)]
assert "a" in keys and "z" not in keys
assert 1 in values and 9 not in values
assert ("a", 1) in items and ("a", 2) not in items
print("views:", keys, values, items)

assert (type(keys).__name__, type(values).__name__, type(items).__name__) == (
    "dict_keys",
    "dict_values",
    "dict_items",
)
assert (keys == keys, values == values, counts.values() == counts.values()) == (
    True,
    True,
    False,
)

# A view reads the dict when it is used: it is not a snapshot.
counts["c"] = 3
assert list(keys) == ["a", "b", "c"]
assert list(values) == [1, 2, 3]
assert len(items) == 3
del counts["a"]
assert list(keys) == ["b", "c"]
print("views are live:", keys)

assert dict(items) == {"b": 2, "c": 3}
total = 0
for value in values:
    total += value
assert total == 5
print("views round-trip: dict(items) and for-over-values")

# ── bitwise operators ─────────────────────────────────────────────────────

assert (6 & 3, 6 | 3, 6 ^ 3, ~6) == (2, 7, 5, -7)
assert (1 << 4, 32 >> 2, -8 >> 1) == (16, 8, -4)
assert (1 | 2 & 3) == 3 and (1 ^ 2 | 4) == 7 and (1 << 2 + 1) == 8
assert ~-1 == 0
# Right shift is arithmetic and saturates; left shift never wraps silently.
assert (1 >> 100, -1 >> 100, 0 << 70) == (0, -1, 0)
# bool/bool keeps bool; bool/int is an int (CPython 3.13+).
assert type(True | False).__name__ == "bool"
assert type(True | 1).__name__ == "int"
number = 5
number |= 2
number <<= 1
assert number == 14
print("bitwise:", 6 & 3, 6 | 3, 6 ^ 3, ~6, 1 << 4, 32 >> 2)

shift_overflow = ""
try:
    1 << 64
    raise AssertionError("1 << 64 must overflow the 64-bit int")
except OverflowError as error:
    shift_overflow = str(error)
assert shift_overflow == "integer left shift overflow"

negative_shift = ""
try:
    1 << -1
    raise AssertionError("a negative shift count must be a ValueError")
except ValueError as error:
    negative_shift = str(error)
assert negative_shift == "negative shift count"

type_error = ""
try:
    "a" & 1
    raise AssertionError("str & int must be a TypeError")
except TypeError as error:
    type_error = str(error)
assert type_error == "unsupported operand type(s) for &: 'str' and 'int'"
print("bitwise errors:", shift_overflow, "|", negative_shift, "|", type_error)

# ── set operations ────────────────────────────────────────────────────────

a = {1, 2, 3}
b = {3, 4}
assert (a | b, a & b, a - b, a ^ b) == ({1, 2, 3, 4}, {3}, {1, 2}, {1, 2, 4})
assert (a | b) == {1, 2, 3, 4}
print("set operators:", a | b, a & b, a - b, a ^ b)

# In-place operators mutate the set object: an alias sees the update.
set_a = {1, 2}
alias = set_a
set_a |= {3}
assert (set_a, alias) == ({1, 2, 3}, {1, 2, 3})
set_a &= {2, 3, 4}
assert (set_a, alias) == ({2, 3}, {2, 3})
set_a -= {2}
assert (set_a, alias) == ({3}, {3})
set_a ^= {3, 4}
assert (set_a, alias) == ({4}, {4})
print("in-place set operators: aliases see the update")

# The mutating methods return None and take any iterables.
target = {1, 2}
assert target.update([3], {4}) is None
assert target == {1, 2, 3, 4}
assert target.difference_update([1]) is None
assert target == {2, 3, 4}
assert target.intersection_update([2, 3, 5]) is None
assert target == {2, 3}
assert target.symmetric_difference_update([3, 6]) is None
assert target == {2, 6}
print("set update methods:", target)

assert {1, 2, 3}.symmetric_difference([3, 4]) == {1, 2, 4}
assert {1, 2}.isdisjoint([3, 4]) is True
assert {1, 2}.isdisjoint([2, 3]) is False
assert {1, 2}.issubset([1, 2, 3]) is True
assert {1, 2, 3}.issuperset([1, 2]) is True
popped = {7, 8}.pop()
assert popped == 7
print("symmetric_difference/isdisjoint/subset/popping worked")

empty_pop = ""
try:
    set().pop()
    raise AssertionError("pop() from an empty set must be a KeyError")
except KeyError as error:
    empty_pop = str(error)
assert empty_pop == "pop from an empty set"

operand_error = ""
try:
    {1} | [2]
    raise AssertionError("set | list must be a TypeError")
except TypeError as error:
    operand_error = str(error)
assert operand_error == "unsupported operand type(s) for |: 'set' and 'list'"
print("set errors:", empty_pop, "|", operand_error)

print("DICT SET OK")