# F-r4-rt-2: 350 nested lists compared inside a generator. On stock 0.9.3 the
# unbounded native recursion (cmp_values -> compare_sequences -> cmp_values)
# overran the 64 KiB heap fiber into the live interpreter heap and the process
# died with `load page fault (stval=0x10)`. The fixed interpreter must print a
# clean RecursionError and exit 1 with the machine still alive.
def deep(n):
    a = []
    for _ in range(n):
        a = [a]
    return a


def gen(n):
    a = deep(n)
    b = deep(n)
    print("BUILT", n)
    r = a < b
    print("CMP", r, "SURVIVED", n)
    yield 1


for _v in gen(350):
    pass
print("DEPTH 350 ALIVE")