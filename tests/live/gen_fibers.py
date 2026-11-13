# Live proof for the fiber limits of pythonc 0.7.0: every suspended generator
# owns one 64 KiB fiber stack, at most 8 may be suspended at once (the ninth
# gets a Python MemoryError, not an allocator abort), and finished/closed
# generators return their stack to the pool. The generator protocol itself is
# tests/live/gen_methods.py and tests/live/gen_delegation.py.
# Run: run /mnt/pythonc /mnt/gen_fibers.py
#
# This is its own script on purpose: the 8-stack limit needs 512 KiB of the
# kernel's 1 MiB bump heap, so the AST of the script sharing the process must
# stay small.


def counter():
    total = 0
    while True:
        value = yield total
        total += value

def one():
    yield 1

# 200 generators come and go through send/close, so no fiber slot leaks.
for _ in range(200):
    g = counter()
    next(g)
    g.send(1)
    g.close()
print("recycled: 200 generators")

# The documented limit stays true: 8 suspended generators, the ninth gets a
# Python MemoryError instead of an allocator abort.
suspended = []
for _ in range(8):
    g = one()
    next(g)
    suspended.append(g)
limit = ""
try:
    ninth = one()
    next(ninth)
    raise AssertionError("the ninth suspended generator must fail")
except MemoryError as error:
    limit = str(error)
assert "8 are already suspended" in limit
for g in suspended:
    g.close()
probe = one()
assert next(probe) == 1
probe.close()
print("limit:", limit)

print("GEN FIBERS OK")
