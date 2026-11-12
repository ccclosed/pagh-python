# Live proof for `yield from` delegation in pythonc 0.7.0: `send`, `throw` and
# `close` are routed to the delegated generator, the delegated expression takes
# the delegate's `return` value, and the 0.5.0 fiber limits (8 suspended
# generators, recycled stacks) still hold. The direct methods are
# tests/live/gen_methods.py.
# Run: run /mnt/pythonc /mnt/gen_delegation.py


def sub():
    while True:
        value = yield
        if value == "stop":
            return "sub return"

def outer():
    result = yield from sub()
    yield result

g = outer()
assert next(g) is None
assert g.send("a") is None
assert g.send("stop") == "sub return"
g.close()
print("delegate send: sub return")

def sub_throw():
    try:
        yield 1
    except ValueError as error:
        yield "handled " + str(error)

def outer_throw():
    yield from sub_throw()

g = outer_throw()
assert next(g) == 1
assert g.throw(ValueError("bad")) == "handled bad"
g.close()
print("delegate throw: handled bad")

# close closes the delegate first, then raises GeneratorExit at the outer
# `yield from`, so both `finally` blocks run, inner first.
log = []

def inner():
    try:
        yield 10
    finally:
        log.append("inner")

def wrapper():
    try:
        yield from inner()
    finally:
        log.append("outer")

g = wrapper()
assert next(g) == 10
assert g.close() is None
assert log == ["inner", "outer"]
print("delegate close:", log)

# The value of a delegated expression is the delegate's `return` value.
def with_return():
    yield 1
    return "answer"

def capture():
    got = yield from with_return()
    yield got

assert list(capture()) == [1, "answer"]
print("delegate return value: answer")

# Delegation nests: a send travels down two `yield from` levels.
def level1():
    value = yield "deep"
    yield ("level1", value)

def level2():
    yield from level1()

def level3():
    yield from level2()

g = level3()
assert next(g) == "deep"
assert g.send("down") == ("level1", "down")
g.close()
print("delegate nested: deep level1 down")

# A plain iterator has no send: the value is refused with AttributeError,
# while throw/close are handled at the outer `yield from` point.
def from_list():
    try:
        yield from [1, 2]
    except AttributeError as error:
        yield "send" in str(error)

g = from_list()
next(g)
assert g.send("x") is True
g.close()
print("delegate plain send: AttributeError")

def from_list_throw():
    try:
        yield from [1, 2]
    except ValueError as error:
        yield "caught " + str(error)

g = from_list_throw()
next(g)
assert g.throw(ValueError("plain")) == "caught plain"
g.close()
print("delegate plain throw: caught plain")

# ── fibers: the 8-suspended limit and stack recycling still hold ──────────

def counter():
    total = 0
    while True:
        value = yield total
        total += value

def one():
    yield 1

# 200 generators come and go through send/close, so no fiber slot leaks.
for _ in range(200):
    g = counter()
    next(g)
    g.send(1)
    g.close()
print("recycled: 200 generators")

# The documented limit stays true: 8 suspended generators, the ninth gets a
# Python MemoryError instead of an allocator abort.
suspended = []
for _ in range(8):
    g = one()
    next(g)
    suspended.append(g)
limit = ""
try:
    ninth = one()
    next(ninth)
    raise AssertionError("the ninth suspended generator must fail")
except MemoryError as error:
    limit = str(error)
assert "8 are already suspended" in limit
for g in suspended:
    g.close()
probe = one()
assert next(probe) == 1
probe.close()
print("limit:", limit)

print("GEN DELEGATION OK")
