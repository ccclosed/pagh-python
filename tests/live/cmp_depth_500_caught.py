# F-r4-rt-2: the 500-level comparison (depth 500 on stock 0.9.3 went below
# the heap base, `store page fault stval=0x1fffffd8`) is caught inside the same
# generator and the interpreter stays usable. A self-referential list is also
# stopped by the depth cap instead of recursing forever, and ordinary
# comparisons keep working afterwards.
def deep(n):
    a = []
    for _ in range(n):
        a = [a]
    return a


def gen(n):
    a = deep(n)
    b = deep(n)
    print("BUILT", n)
    caught = False
    try:
        r = a < b
    except RecursionError:
        caught = True
    print("CMP BOMB CAUGHT", caught)
    print("CMP RECOVERED", [1, 2] < [1, 3])
    yield caught


canary = [123456789, [7, 8, 9], "heap-canary"]
assert list(gen(500)) == [True]
assert canary == [123456789, [7, 8, 9], "heap-canary"]
print("CMP HEAP CANARY OK")

cyclic = []
cyclic.append(cyclic)
cyclic_caught = False
try:
    cyclic < cyclic
except RecursionError:
    cyclic_caught = True
print("CMP CYCLIC CAUGHT", cyclic_caught)
assert cyclic_caught
assert canary == [123456789, [7, 8, 9], "heap-canary"]
print("CMP HEAP STILL OK")
print("ALIVE AFTER CMP BOMB")