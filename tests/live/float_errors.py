import math
checks = 0
try:
    int(float("nan"))
    assert False
except ValueError:
    checks += 1
try:
    int(float("inf"))
    assert False
except OverflowError:
    checks += 1
try:
    math.sqrt(-1)
    assert False
except ValueError:
    checks += 1
try:
    0.0 ** -1
    assert False
except ZeroDivisionError:
    checks += 1
try:
    (-8) ** (1 / 3)
    assert False
except ValueError:
    checks += 1
try:
    math.log(0)
    assert False
except ValueError:
    checks += 1
print("typed float errors:", checks)
assert checks == 6
print("float errors ok")
