def make_adder(n):
    return lambda x, y=0: x + y + n

add5 = make_adder(5)
print("lambda defaults:", add5(10), add5(10, 1))

double = lambda value: value * 2
print("lambda map:", list(map(double, [1, 2, 3])))

words = ["pagh", "os", "interpreter"]
print("sorted key:", sorted(words, key=lambda word: len(word)))

pairs = [("b", 2), ("a", 1), ("c", 3)]
pairs.sort(key=lambda pair: pair[1])
print("sort key:", pairs)

identity = lambda value: value
assert add5(10) == 15
assert add5(10, 1) == 16
assert double(21) == 42
assert list(map(lambda a, b: a + b, [1, 2], [10, 20, 30])) == [11, 22]
assert sorted(words, key=lambda word: len(word)) == ["os", "pagh", "interpreter"]
assert identity("x") == "x"
print("lambda ok")
