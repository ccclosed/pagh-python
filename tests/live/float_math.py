import math
import sys
print("pi/e:", math.pi, math.e)
print("sqrt/pow:", math.sqrt(2), math.pow(2, 0.5))
print("trig:", math.sin(1), math.cos(1), math.tan(1))
print("log/exp:", math.log(math.e), math.log10(1000), math.exp(1))
print("rounding:", math.floor(-2.1), math.ceil(-2.1), math.trunc(-2.9), math.fabs(-2.5))
assert math.pi == 3.141592653589793
assert math.e == 2.718281828459045
assert math.sqrt(2) == 1.4142135623730951
assert math.sqrt(4) == 2.0
assert math.floor(-2.1) == -3
assert math.ceil(-2.1) == -2
assert math.trunc(-2.9) == -2
assert math.fabs(-2.5) == 2.5
assert math.fmod(-7.5, 2) == -1.5
assert math.pow(-2, 3) == -8.0
assert math.sin(0) == 0.0
assert math.cos(0) == 1.0
assert math.tan(0) == 0.0
assert abs(math.sin(1) - 0.8414709848078965) < 1e-15
assert abs(math.cos(1) - 0.5403023058681398) < 1e-15
assert abs(math.tan(1) - 1.5574077246549023) < 1e-15
assert abs(math.log(math.e) - 1.0) < 1e-15
assert abs(math.log10(1000) - 3.0) < 1e-15
assert math.log(8, 2) == 3.0
assert abs(math.exp(1) - 2.718281828459045) < 1e-15
assert math.isnan(math.nan)
assert math.isinf(math.inf)
assert not math.isinf(1.0)
assert sys.float_info.dig == 15
print("float math ok")
