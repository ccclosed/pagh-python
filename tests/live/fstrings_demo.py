name = "pagh"
pi = 3.14159
count = 42
print(f"hello {name}")
print(f"pi={pi:.2f} count={count:d}")
print(f"repr={name!r} width={count:>5d}")
print(f"braces {{ok}}")
assert f"hello {name}" == "hello pagh"
assert f"{pi:.2f}" == "3.14"
assert f"{count:d}" == "42"
assert f"{name!r}" == repr(name)
assert f"{{x}}" == "{x}"
assert f"{5:>3d}" == "  5"
text = f"sum={2 + 3}"
assert text == "sum=5"
print("fstrings ok")
