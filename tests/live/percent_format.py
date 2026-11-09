# Live proof for printf-style `%` string formatting in pythonc 0.5.0.
# Run: run /mnt/pythonc /mnt/percent_format.py
#
# The conversion shares the f-string formatter, so widths, signs and precision
# behave like the equivalent replacement field.

name = "pagh"
count = 42
print("basic:", "%s and %d" % (name, count))
assert "%s and %d" % (name, count) == "pagh and 42"

print("widths:", "%5d|%-5d|%05d" % (42, 42, 42))
assert "%5d|%-5d|%05d" % (42, 42, 42) == "   42|42   |00042"

print("floats:", "%.2f|%8.3f|%+d" % (3.14159, 3.14159, 7))
assert "%.2f|%8.3f|%+d" % (3.14159, 3.14159, 7) == "3.14|   3.142|+7"

print("radix:", "%x %X %o %b" % (255, 255, 8, 5))
assert "%x %X %o %b" % (255, 255, 8, 5) == "ff FF 10 101"

print("repr:", "%r %s" % ("quo'te", 5))
assert "%r" % "quo'te" == "\"quo'te\"" or "%r" % "quo'te" == "'quo\\'te'"

print("percent:", "%d%%" % 50)
assert "%d%%" % 50 == "50%"

print("char:", "%c%c" % (65, "b"))
assert "%c%c" % (65, "b") == "Ab"

print("single:", "%s" % [1, 2], "|", "%s" % ((1, 2),))
assert "%s" % [1, 2] == "[1, 2]"
assert "%s" % ((1, 2),) == "(1, 2)"

print("scientific:", "%e" % 12345.678, "%g" % 0.0000123, "%.1f" % 2.25)
assert "%e" % 12345.678 == "1.234568e+04"

print("signs:", "%+d % d" % (5, 5))
assert "%+d % d" % (5, 5) == "+5  5"

print("nested:", "%s" % ("%d" % 3))
assert "%s" % ("%d" % 3) == "3"

failed = ""
try:
    "%d" % "text"
except TypeError:
    failed = "type"
print("bad type:", failed)
assert failed == "type"

failed = ""
try:
    "%d %d" % (1,)
except TypeError:
    failed = "missing"
print("missing arg:", failed)
assert failed == "missing"

failed = ""
try:
    "%d" % (1, 2)
except TypeError:
    failed = "extra"
print("extra arg:", failed)
assert failed == "extra"

failed = ""
try:
    "%q" % 1
except ValueError:
    failed = "character"
print("bad conversion:", failed)
assert failed == "character"

# `%` between numbers is still modulo.
print("modulo:", 7 % 3, 7.5 % 2)
assert (7 % 3) == 1 and (7.5 % 2) == 1.5

print("PERCENT OK")
