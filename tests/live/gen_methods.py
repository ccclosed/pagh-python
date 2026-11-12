# Live proof for the generator protocol of pythonc 0.7.0: `send`, `throw` and
# `close`. The same three methods delegated through `yield from`, plus the
# fiber limits, are tests/live/gen_delegation.py (one script per boot keeps the
# 1 MiB heap honest: the AST of a bigger script is itself memory).
# Run: run /mnt/pythonc /mnt/gen_methods.py
#
# The generator machinery is the 0.5.0 fiber one: these methods resume the same
# suspended stack, so the point of this script is that the value/exception
# actually crosses the `yield` (and a nested `yield from`) at the right place,
# including the `finally` blocks.

# ── send ───────────────────────────────────────────────────────────────────

def echo():
    first = yield "ready"
    second = yield first
    return second

g = echo()
a = g.send(None)
b = g.send("one")
stopped = False
try:
    g.send("two")
except StopIteration:
    stopped = True
assert (a, b, stopped) == ("ready", "one", True)
print("send:", a, b, stopped)
del g

g = echo()
try:
    g.send("nope")
    raise AssertionError("send into a fresh generator must be a TypeError")
except TypeError as error:
    message = str(error)
assert message == "can't send non-None value to a just-started generator"
print("send fresh:", message)
del g

# A generator driven entirely by send: the value replaces `yield`'s result.
def counter():
    total = 0
    while True:
        value = yield total
        total += value

g = counter()
assert next(g) == 0
assert g.send(2) == 2
assert g.send(3) == 5
g.close()
print("send driven: 0 2 5")

# Sending into an exhausted generator is a StopIteration, not a crash.
g = echo()
list(g)
try:
    g.send(1)
    raise AssertionError("send into an exhausted generator must stop")
except StopIteration:
    pass
print("send exhausted: StopIteration")
del g

# ── throw ──────────────────────────────────────────────────────────────────

def catcher():
    try:
        yield "ready"
    except KeyError:
        yield "key"
    except ValueError as error:
        yield "value:" + str(error)

g = catcher()
assert next(g) == "ready"
assert g.throw(ValueError("boom")) == "value:boom"
g.close()
print("throw instance: value:boom")

g = catcher()
next(g)
assert g.throw(KeyError) == "key"
g.close()

g = catcher()
next(g)
assert g.throw(ValueError, "msg") == "value:msg"
g.close()
print("throw class / class+args: key, value:msg")

# A user-defined exception class works through throw, as instance and as
# class + arguments.
class MyError(Exception):
    pass

def user_catcher():
    try:
        yield "ready"
    except MyError as error:
        yield "mine:" + str(error)

g = user_catcher()
next(g)
assert g.throw(MyError("custom")) == "mine:custom"
g.close()
g = user_catcher()
next(g)
assert g.throw(MyError, "made") == "mine:made"
g.close()
print("throw user class: mine:custom mine:made")

# An uncaught throw escapes from the generator to its resumer.
def plain():
    yield 1

g = plain()
next(g)
escaped = ""
try:
    g.throw(RuntimeError("escaping"))
    raise AssertionError("an uncaught throw must propagate")
except RuntimeError as error:
    escaped = str(error)
assert escaped == "escaping"
assert next(g, "done") == "done"
print("throw escapes:", escaped)
del g

# Throwing into a generator that never started does not run its body.
log = []

def guarded_throw():
    try:
        yield 1
    finally:
        log.append("finally")

g = guarded_throw()
try:
    g.throw(RuntimeError("early"))
except RuntimeError:
    pass
assert log == []
assert next(g, "done") == "done"
print("throw unstarted: body not run")
del g

# ── close ──────────────────────────────────────────────────────────────────

log = []

def guarded():
    try:
        yield 1
        yield 2
    finally:
        log.append("cleanup")

g = guarded()
assert next(g) == 1
assert g.close() is None
assert log == ["cleanup"]
assert next(g, "done") == "done"
print("close finally:", log)

# Closing a generator that never started runs nothing at all.
log = []
g = guarded()
assert g.close() is None
assert log == []
assert next(g, "done") == "done"
del g
print("close unstarted: nothing ran")

# A body that swallows GeneratorExit and yields again is refused.
def stubborn():
    try:
        yield 1
    except GeneratorExit:
        yield 2

g = stubborn()
next(g)
refused = ""
try:
    g.close()
    raise AssertionError("close must refuse a generator that yields again")
except RuntimeError as error:
    refused = str(error)
assert refused == "generator ignored GeneratorExit"
g.close()
print("close ignored:", refused)

# GeneratorExit is a BaseException, not an Exception, so `except Exception`
# does not swallow a close; a body that catches it and returns closes cleanly.
assert isinstance(GeneratorExit(), BaseException)
assert not isinstance(GeneratorExit(), Exception)

def catches():
    try:
        yield 1
    except GeneratorExit:
        return

g = catches()
next(g)
assert g.close() is None
print("close: GeneratorExit is BaseException")

print("GEN METHODS OK")
