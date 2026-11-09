# Live proof for the os.path / time / random modules in pythonc 0.5.0.
# Run: run /mnt/pythonc /mnt/stdlib_modules.py
#
# os.path wraps the ABI's single path separator and the kernel's stat; time
# uses the two ABI clocks and nanosleep; random is a deterministic xorshift64*
# generator, so the same script prints the same numbers on every boot.

import os.path
import sys
from os.path import join, basename, dirname, exists, isfile, isdir

print("join:", join("a", "b", "c"), join("/root", "x"), join("a", "/abs", "b"))
assert join("a", "b", "c") == "a/b/c"
assert join("/root", "x") == "/root/x"
assert join("a", "/abs", "b") == "/abs/b"
assert join("dir/", "file") == "dir/file"

print("split:", basename("/mnt/test.py"), dirname("/mnt/test.py"), basename("x"), dirname("x"))
assert basename("/mnt/test.py") == "test.py"
assert dirname("/mnt/test.py") == "/mnt"
assert basename("/") == "" and dirname("/") == "/"
assert dirname("a/b/") == "a"

print("sep:", os.path.sep)
assert os.path.sep == "/"

# Probe the script itself, so the check points at a real file in both the
# guest and the host dry-run.
script = sys.argv[0]
assert exists(script)
assert isfile(script)
assert not isdir(script)
assert basename(script) == "stdlib_modules.py"
assert isdir(dirname(script))
assert exists(join(dirname(script), "stdlib_modules.py"))
assert not exists(join(dirname(script), "definitely-missing"))
print("stat probes:", script, exists(script), isdir(script))

import time

start = time.monotonic()
time.sleep(0.05)
elapsed = time.monotonic() - start
# The ABI clock has 1 ms resolution and the kernel rounds a sleep up to the
# next 10 ms tick, so compare with a tolerance rather than exact float equality
# (0.05 is not representable, and the difference of two clock readings can land
# a few ulp below it).
print("sleep:", elapsed, elapsed >= 0.045)
assert elapsed >= 0.045

wall = time.time()
print("clocks:", wall > 0, time.monotonic() >= 0)
assert wall > 0
assert time.monotonic() >= 0

negative = ""
try:
    time.sleep(-1)
except ValueError:
    negative = "rejected"
print("negative sleep:", negative)
assert negative == "rejected"

import random

random.seed(7)
first = [random.random() for _ in range(3)]
random.seed(7)
second = [random.random() for _ in range(3)]
print("deterministic:", first == second, first)
assert first == second

unit = random.random()
assert 0.0 <= unit < 1.0
print("unit interval:", unit >= 0.0 and unit < 1.0)

random.seed(1)
rolls = [random.randint(1, 6) for _ in range(200)]
print("randint:", min(rolls), max(rolls), len(rolls))
assert min(rolls) >= 1 and max(rolls) <= 6 and len(rolls) == 200

random.seed(3)
fixed = [random.randint(5, 5) for _ in range(4)]
print("degenerate randint:", fixed)
assert fixed == [5, 5, 5, 5]

random.seed(2)
picked = random.choice("abc")
print("choice:", picked)
assert picked in "abc"

random.seed(11)
shuffled = [1, 2, 3, 4, 5]
random.shuffle(shuffled)
print("shuffle:", shuffled, sorted(shuffled))
assert sorted(shuffled) == [1, 2, 3, 4, 5]

random.seed(5)
uniform = random.uniform(1.0, 2.0)
print("uniform:", uniform)
assert 1.0 <= uniform <= 2.0

empty = ""
try:
    random.choice([])
except IndexError:
    empty = "IndexError"
print("empty choice:", empty)
assert empty == "IndexError"

print("STDLIB OK")
