# Live proof for generator expressions in pythonc 0.5.0.
# Run: run /mnt/pythonc /mnt/genexpr.py
#
# `(element for target in iterable ...)` builds a lazy generator, not a list:
# the same expression over an unbounded source terminates only because nothing
# forces the values that are not asked for.

squares = list(value * value for value in range(5))
print("squares:", squares)
assert squares == [0, 1, 4, 9, 16]

evens = sum(value for value in range(10) if value % 2 == 0)
print("filtered sum:", evens)
assert evens == 20

print("type:", type(value for value in range(2)).__name__)
assert type(value for value in range(2)).__name__ == "generator"

pairs = list((a, b) for a in range(2) for b in range(2))
print("nested loops:", pairs)
assert pairs == [(0, 0), (0, 1), (1, 0), (1, 1)]

# Laziness over an unbounded generator function.
def naturals():
    n = 0
    while True:
        yield n
        n += 1

stream = (value * 2 for value in naturals())
taken = [next(stream) for _ in range(4)]
print("lazy:", taken)
assert taken == [0, 2, 4, 6]
del stream

# The bare form is allowed as a call's only argument.
total = sum(value * 3 for value in range(4))
print("bare in call:", total)
assert total == 18

# A generator expression captures the enclosing scope when it is created.
def make(limit):
    return (value for value in range(limit))

made = make(3)
print("closure:", list(made))
assert list(make(3)) == [0, 1, 2]

# It is consumed exactly once.
consumed = (value for value in range(3))
print("single pass:", list(consumed), list(consumed))
assert list(consumed) == []

print("GENEXPR OK")
