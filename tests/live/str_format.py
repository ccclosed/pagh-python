# Live proof for str.format in pythonc 0.4.0.
# Run: run /mnt/pythonc /mnt/str_format.py
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

values = {"name": "pagh", "n": 7}
print("auto:", "{} + {} = {}".format(2, 3, 5))
print("manual:", "{1}/{0}/{1}".format("a", "b"))
print("named:", "{name}-{n}".format(**values))
print("escape:", "{{literal}} {} {}".format("{", "}"))
print("spec:", "{:04d} {:.3f} {:>5}".format(7, 3.14159, "ok"))
print("index:", "{0[1]}".format(["zero", "one"]))
print("key:", "{d[k]}".format(d={"k": "v"}))
p = Point(2, 9)
print("attr:", "{p.x},{p.y}".format(p=p))
print("conversion:", "{0!r}".format("hi"))

assert "{}".format(42) == "42"
assert "{0}-{1}".format("a", "b") == "a-b"
assert "{{}}".format() == "{}"
assert "{:.2f}".format(2.5) == "2.50"
assert "{x}".format(x=9) == "9"
assert "{0[1]}".format([3, 4]) == "4"
assert "{p.x}".format(p=p) == "2"
print("FORMAT OK")
