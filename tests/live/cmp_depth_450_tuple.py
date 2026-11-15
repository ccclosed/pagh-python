# F-r4-rt-2: the tuple variant at 450 levels, uncaught. tuple comparison took
# the same unbounded native path on stock 0.9.3 (heap corruption, then a
# wild-pointer load fault); now it is a clean RecursionError, exit 1.
def deep_tuple(n):
    a = ()
    for _ in range(n):
        a = (a,)
    return a


def gen(n):
    a = deep_tuple(n)
    b = deep_tuple(n)
    print("BUILT TUPLE", n)
    r = a < b
    print("CMP", r, "SURVIVED", n)
    yield 1


for _v in gen(450):
    pass
print("DEPTH 450 TUPLE ALIVE")