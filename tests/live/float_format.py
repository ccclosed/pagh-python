print("str:", str(1.0), str(1.5), str(1e20), str(1e-7), str(1.5e-7))
print("repr:", repr(1.0), repr(0.1 + 0.2), repr(-0.0))
assert str(1.0) == "1.0"
assert repr(1.0) == "1.0"
assert str(1e20) == "1e+20"
assert repr(1e-7) == "1e-07"
assert str(1.5e-7) == "1.5e-07"
assert str(0.1 + 0.2) == "0.30000000000000004"
assert str(-0.0) == "-0.0"
assert round(2.5) == 2
assert round(3.5) == 4
assert round(-2.5) == -2
assert round(2.675, 2) == 2.67
assert round(1.25, 1) == 1.2
assert round(1234, -2) == 1200
assert type(1.0).__name__ == "float"
print("float formatting ok")
