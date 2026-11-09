# Live proof for decorators in pythonc 0.5.0.
# Run: run /mnt/pythonc /mnt/decorators.py
#
# `@decorator` is applied at definition time, bottom-up, and the name binds
# whatever the decorator returns (function, wrapper, value or class).

def double(function):
    def wrapper(*args, **kwargs):
        return function(*args, **kwargs) * 2
    return wrapper

@double
def five():
    return 5

print("double:", five())
assert five() == 10

order = []

def mark(label):
    def decorate(function):
        def wrapper(*args, **kwargs):
            order.append(label)
            return function(*args, **kwargs)
        return wrapper
    return decorate

@mark("top")
@mark("bottom")
def target():
    return "value"

result = target()
print("order:", result, order)
assert result == "value"
assert order == ["top", "bottom"]

# A decorator may replace the function with a plain value.
def constant(function):
    return 42

@constant
def ignored():
    return 1

print("replaced:", ignored)
assert ignored == 42

# A decorator that returns the function unchanged keeps the signature.
def passthrough(function):
    return function

@passthrough
def add(a, b=2):
    return a + b

print("signature:", add(1), add(1, 5))
assert add(1) == 3 and add(1, 5) == 6

# Methods are decorated like any other function and still receive self.
class Point:
    def __init__(self, x):
        self.x = x

    @double
    def scaled(self):
        return self.x

print("method:", Point(3).scaled())
assert Point(3).scaled() == 6

# A decorated generator function is still a generator function.
@passthrough
def counter(n):
    for value in range(n):
        yield value

print("generator decorator:", list(counter(3)))
assert list(counter(3)) == [0, 1, 2]

# @staticmethod: no implicit self, callable from the class and the instance.
class Tools:
    @staticmethod
    def add(a, b):
        return a + b

    def twice(self, value):
        return value * 2

print("staticmethod:", Tools.add(1, 2), Tools().add(3, 4), Tools().twice(5))
assert Tools.add(1, 2) == 3
assert Tools().add(3, 4) == 7
assert Tools().twice(5) == 10

# Classes can be decorated too.
def rename(cls):
    cls.label = "decorated"
    return cls

@rename
class Thing:
    pass

print("class decorator:", Thing.label)
assert Thing.label == "decorated"

print("DECORATORS OK")
