# Live proof for `yield from` delegation in pythonc 0.6.0.
# Run: run /mnt/pythonc /mnt/yield_from.py
#
# `yield from` is the iteration protocol: the operand is evaluated once, then
# every value it produces is yielded on to the resumer. A delegated generator
# is resumed from inside the outer generator's fiber, so laziness, the fiber
# stack pool and the "no more than 8 suspended generators" limit are exactly
# the 0.5.0 ones.

def counter(n):
    i = 0
    while i < n:
        yield i
        i += 1

# The delegated values come out between the outer generator's own yields.
def outer():
    yield "before"
    yield from counter(3)
    yield "after"

print("outer:", list(outer()))
assert list(outer()) == ["before", 0, 1, 2, "after"]

# Any iterable works, not only a generator: range, list, generator
# expression, and an object implementing __iter__.
def from_range():
    yield from range(3)

def from_list():
    yield from [7, 8]

def from_genexp():
    yield from (v * v for v in range(3))

class Box:
    def __iter__(self):
        return iter(["a", "b"])

def from_box():
    yield from Box()

print("range:", list(from_range()), list(from_list()), list(from_genexp()), list(from_box()))
assert list(from_range()) == [0, 1, 2]
assert list(from_list()) == [7, 8]
assert list(from_genexp()) == [0, 1, 4]
assert list(from_box()) == ["a", "b"]

# The expression evaluates to None (there is no generator return value), so
# the name after `=` holds None when delegation finishes.
def captured():
    got = yield from [1, 2]
    yield got

assert list(captured()) == [1, 2, None]
print("value:", list(captured()))

# Delegation nests to any depth: each level is resumed from the previous one.
def first():
    yield 1
    yield 2

def second():
    yield from first()

def third():
    yield from second()

print("nested:", list(third()))
assert list(third()) == [1, 2]

# Laziness: an infinite delegated source is pulled one value at a time, so a
# `break` in the consumer ends it.
def naturals():
    n = 0
    while True:
        yield n
        n += 1

def delegated_naturals():
    yield from naturals()

stream = delegated_naturals()
taken = []
for value in stream:
    taken.append(value)
    if value == 4:
        break
print("lazy:", taken)
assert taken == [0, 1, 2, 3, 4]
del stream

# A delegated generator that finishes mid-way lets the outer body continue.
events = []

def guarded():
    try:
        yield 1
        yield 2
    finally:
        events.append("cleanup")

def wrapper():
    yield "start"
    yield from guarded()
    yield "end"

print("finally:", list(wrapper()), events)
assert list(wrapper()) == ["start", 1, 2, "end"]
assert events == ["cleanup", "cleanup"]

# A non-iterable operand is a TypeError raised from the first next().
def broken():
    yield from 5

try:
    list(broken())
    print("non-iterable: NO ERROR")
    raise AssertionError("yield from 5 must raise TypeError")
except TypeError as error:
    print("non-iterable:", type(error).__name__, "not iterable" in str(error))
    assert "not iterable" in str(error)

# Delegating to someone else's generator does not consume it twice: the
# consumer and the delegate are the same iterator chain.
def single():
    yield 1
    yield 2

shared = single()
def passthrough(gen):
    yield from gen

seen = list(passthrough(shared))
again = list(shared)
print("consumed once:", seen, again)
assert seen == [1, 2] and again == []

# Fiber stacks are recycled: 200 delegated generators come and go in a loop.
total = 0
for _ in range(200):
    for value in from_range():
        total += value
print("recycled:", total)
assert total == 600

print("YIELD FROM OK")
