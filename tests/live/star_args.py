# Live proof for *args / **kwargs in pythonc 0.4.0.
# Run: run /mnt/pythonc /mnt/star_args.py
def collect(*args, **kwargs):
    return args, kwargs

print("collect:", collect(1, "two", True, name="pagh", n=3))

def mixed(a, b=20, *rest, **extra):
    return a, b, rest, extra

print("mixed-default:", mixed(1))
print("mixed-full:", mixed(1, 2, 3, 4, z=5))

def add(a, b):
    return a + b

print("star-list:", add(*[10, 20]))
print("star-dict:", add(**{"a": 30, "b": 40}))
print("both:", add(*[1], **{"b": 2}))

def forward(*args, **kwargs):
    return mixed(*args, **kwargs)

print("forward:", forward(1, 2, 3, 4, z=5))

pair = lambda *args, **kwargs: (args, kwargs)
print("lambda:", pair(7, k=8))

assert collect() == ((), {})
assert collect(1, 2, a=3) == ((1, 2), {"a": 3})
assert add(*[1, 2]) == 3
assert add(**{"a": 4, "b": 5}) == 9
assert mixed(1, 2, 3, q=4) == (1, 2, (3,), {"q": 4})
print("STARARGS OK")
