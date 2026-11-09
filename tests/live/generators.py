# Live proof for generator functions in pythonc 0.5.0.
# Run: run /mnt/pythonc /mnt/generators.py
#
# Generators are real coroutines: the body is suspended on its own stack at
# each `yield`, so laziness, `finally`, nesting and error propagation are the
# property being tested here, not just the values that come out.

def counter(n):
    i = 0
    while i < n:
        yield i
        i += 1

print("list:", list(counter(4)))
assert list(counter(4)) == [0, 1, 2, 3]

gen = counter(3)
print("next:", next(gen), next(gen), next(gen))
assert next(gen, "exhausted") == "exhausted"

stopped = False
try:
    next(gen)
except StopIteration:
    stopped = True
assert stopped
print("stopiteration:", stopped)

total = 0
for value in counter(5):
    total += value
else:
    total += 100
assert total == 110
print("for/else:", total)

# Laziness: this loop never ends, so only a suspended generator finishes it.
def naturals():
    n = 0
    while True:
        yield n
        n += 1

stream = naturals()
first = [next(stream) for _ in range(5)]
print("lazy:", first)
assert first == [0, 1, 2, 3, 4]
del stream

# A `yield` expression evaluates to the value the caller sent (None here).
def echo():
    received = yield "first"
    yield received

talker = echo()
print("yield expr:", next(talker), next(talker))
assert next(echo()) == "first"

# finally runs when the generator is exhausted.
events = []

def guarded():
    try:
        yield "a"
        yield "b"
    finally:
        events.append("cleanup")

values = list(guarded())
print("finally:", values, events)
assert values == ["a", "b"] and events == ["cleanup"]

# An exception inside the generator reaches the caller of next() and ends it.
def boom():
    yield 1
    raise ValueError("generator failed")

bomb = boom()
assert next(bomb) == 1
raised = ""
try:
    next(bomb)
except ValueError as error:
    raised = str(error)
print("raise:", raised)
assert raised == "generator failed"
assert next(bomb, "done") == "done"

# Generators nest: the outer body resumes the inner one.
def inner():
    yield 1
    yield 2

def outer():
    for value in inner():
        yield value * 10

print("nested:", list(outer()))
assert list(outer()) == [10, 20]

# A generator is its own iterator and reports its type.
one = counter(1)
print("identity:", iter(one) is one, type(one).__name__)
assert iter(one) is one
assert type(one).__name__ == "generator"

# Re-entering a running generator is refused instead of corrupting it.
me = None

def selfish():
    yield next(me)

me = selfish()
message = ""
try:
    next(me)
except ValueError as error:
    message = str(error)
print("reentrant:", message)
assert message == "generator already executing"
del me

# Arguments bind at call time, before the body runs.
def needs(value):
    yield value

arity = ""
try:
    needs()
except TypeError:
    arity = "TypeError"
print("arity:", arity)
assert arity == "TypeError"

# The fiber stack of an exhausted generator is recycled: 200 generators come
# and go in one loop without running out of them.
summed = 0
for _ in range(200):
    for value in counter(4):
        summed += value
print("recycled:", summed)
assert summed == 1200

print("GENERATORS OK")
