# Live proof for keyword-only parameters in pythonc 0.9.0.
# Run: run /mnt/pythonc /mnt/keyword_only.py
#
# `def f(a, *, b)` and `def f(*args, x=1, **kw)` bind the parameters after
# `*`/`*args` by name only: positional arguments never fill them, defaults are
# evaluated once at `def` time, and `**kwargs` collects only the names nothing
# else claimed. Methods, lambdas and generator functions share the binder.

def f(a, *, b, c=3):
    return (a, b, c)

assert f(1, b=2) == (1, 2, 3)
assert f(1, b=2, c=9) == (1, 2, 9)
print("def:", f(1, b=2), f(1, b=2, c=9))

positional = ""
try:
    f(1, 2)
    raise AssertionError("a keyword-only parameter must not accept a positional value")
except TypeError as error:
    positional = str(error)
assert "f() takes 1 positional arguments but 2 were given" == positional
print("positional refused:", positional)

missing = ""
try:
    f(1)
    raise AssertionError("a missing keyword-only argument must be a TypeError")
except TypeError as error:
    missing = str(error)
assert missing == "f() missing 1 required keyword-only argument: 'b'"
print("missing:", missing)

# Bare `*`: no *args binding, everything after it is keyword-only.
def g(*, x, y=2):
    return x + y

assert g(x=5) == 7
assert g(y=0, x=-1) == -1
print("bare star:", g(x=5), g(y=0, x=-1))

# *args + keyword-only + **kwargs in one signature.
def h(*args, x, y=1, **kw):
    return (args, x, y, kw)

assert h(1, 2, x=5, z=9) == ((1, 2), 5, 1, {"z": 9})
assert h(x=0) == ((), 0, 1, {})
assert h(1, x=3, y=4) == ((1,), 3, 4, {})
print("args/kwonly/kwargs:", h(1, 2, x=5, z=9))

# Default ordering: positional defaults before the star do not constrain the
# keyword-only side, where defaults may be interleaved with required names.
def mixed(a=1, *, b, c=2):
    return (a, b, c)

def kworder(*, a=1, b):
    return (a, b)

assert mixed(b=5) == (1, 5, 2)
assert mixed(9, b=5) == (9, 5, 2)
assert kworder(b=2) == (1, 2)
print("defaults:", mixed(b=5), mixed(9, b=5), kworder(b=2))

# Methods get the receiver positionally and the rest keyword-only.
class Box:
    def __init__(self, *, size=1, name="box"):
        self.size = size
        self.name = name

    def scale(self, factor, *, twice=False):
        if twice:
            return self.size * factor * 2
        return self.size * factor

box = Box(size=3, name="cube")
assert box.scale(2) == 6
assert box.scale(2, twice=True) == 12
assert box.name == "cube"
print("method:", box.scale(2), box.scale(2, twice=True))

# Lambdas share the parser and the binder.
assert (lambda *, q: q)(q=7) == 7
assert (lambda a, *, b=2: a + b)(1, b=3) == 4
print("lambda: 7 4")

# Generator functions bind before the first yield, keyword-only included.
def gen(*, n):
    i = 0
    while i < n:
        yield i
        i += 1

assert list(gen(n=3)) == [0, 1, 2]
gen_missing = ""
try:
    list(gen())
    raise AssertionError("a missing keyword-only argument must fail before the body")
except TypeError as error:
    gen_missing = str(error)
assert gen_missing == "gen() missing 1 required keyword-only argument: 'n'"
print("generator:", list(gen(n=3)))

# A mapping unpacked at the call site fills keyword-only parameters.
values = {"a": 1, "b": 2}

def total(*, a, b):
    return a + b

assert total(**values) == 3
assert total(*[], **values) == 3
print("call unpacking:", total(**values))

# Malformed signatures are parser errors and are covered by the host grid
# (there is no `exec()` in this interpreter to try them at run time).

print("KEYWORD ONLY OK")