print("mixed compare:", 1 == 1.0, 1 < 1.5, 2.5 >= 2)
print("nan compare:", float("nan") == float("nan"), float("nan") < 1.0)
assert 1 == 1.0
assert not (1 != 1.0)
assert 1 < 1.5
assert 2.5 >= 2
assert 2 <= 2.0
assert not (9007199254740993 == 9007199254740992.0)
assert 9007199254740993 > 9007199254740992.0
assert not (float("nan") == float("nan"))
assert float("nan") != float("nan")
assert not (float("nan") < 1.0)
assert not (float("nan") > 1.0)
assert min(1, 2.5) == 1
assert max(1, 2.5) == 2.5
assert sorted([3.5, 1, 2.0]) == [1, 2.0, 3.5]
assert sum([1.5, 2.5, 3.0]) == 7.0
print("float comparison ok")
