print("true division:", 1 / 2, 7 / 2, -7 / 2)
print("floor/mod:", 7.0 // 2, -7.0 // 2, 7.5 % 2, -7.5 % 2, 7.5 % -2)
assert 1 / 2 == 0.5
assert 7 / 2 == 3.5
assert -7 / 2 == -3.5
assert 7 // 2 == 3
assert 7.0 // 2 == 3.0
assert -7.0 // 2 == -4.0
assert 7.5 % 2 == 1.5
assert -7.5 % 2 == 0.5
assert 7.5 % -2 == -0.5
assert divmod(7.5, 2) == (3.0, 1.5)
try:
    1 / 0
    assert False
except ZeroDivisionError:
    print("caught integer division by zero")
try:
    1.0 // 0.0
    assert False
except ZeroDivisionError:
    print("caught float floor division by zero")
try:
    1.0 % 0.0
    assert False
except ZeroDivisionError:
    print("caught float modulo by zero")
print("float division ok")
