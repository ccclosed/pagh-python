print("literals:", 1.5, .5, 1e3, 1.5e-2, 1., 1_0.5)
print("mixed:", 1 + 2.5, 2.5 * 2, 5 - 1.25, -1.5, +1.5)
assert 1.5 + 2.25 == 3.75
assert 2.0 * 3 == 6.0
assert 5 - 1.25 == 3.75
assert -1.5 == -(1.5)
assert +1.5 == 1.5
assert abs(-2.5) == 2.5
assert 2 ** -1 == 0.5
assert 2.0 ** 3 == 8.0
assert (-2.0) ** 3 == -8.0
assert 2.0 ** 0.5 == 1.4142135623730951
assert float(3) == 3.0
assert float(True) == 1.0
assert int(3.99) == 3
assert int(-3.99) == -3
assert bool(1.5)
assert not bool(0.0)
assert isinstance(1.5, float)
assert not isinstance(1, float)
print("float arithmetic ok")
