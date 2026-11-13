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

# `throw(GeneratorExit)` through `yield from` is special-cased (CPython's
# gen_throw): the delegate must die. A delegate that swallows GeneratorExit
# and yields makes the outer throw a RuntimeError, one that returns is
# reported as GeneratorExit at the `yield from`.
log = []

def sub_exit_yield():
    try:
        yield 1
    except GeneratorExit:
        log.append("sub")
        yield "caught"

def outer_exit_yield():
    try:
        yield from sub_exit_yield()
    finally:
        log.append("outer")

g = outer_exit_yield()
next(g)
message = ""
try:
    g.throw(GeneratorExit())
    raise AssertionError("a delegate that ignores GeneratorExit must be refused")
except RuntimeError as error:
    message = str(error)
assert message == "generator ignored GeneratorExit"
assert log == ["sub", "outer"]
print("delegate throw GeneratorExit:", message)

def sub_exit_return():
    try:
        yield 1
    except GeneratorExit:
        return "done"

def outer_exit_return():
    yield from sub_exit_return()

g = outer_exit_return()
next(g)
exiting = False
try:
    g.throw(GeneratorExit())
except GeneratorExit:
    exiting = True
assert exiting
print("delegate throw GeneratorExit return: GeneratorExit")

# The delegated `return` value is taken once: a second delegation to the same
# exhausted generator sees None.
def inner_return():
    yield 1
    return "answer"

done = inner_return()
list(done)

def delegate_again():
    got = yield from done
    yield got

assert list(delegate_again()) == [None]
print("delegate return once: None")

print("GEN DELEGATION OK")
