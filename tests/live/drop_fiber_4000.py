# F-r3-rt-1: the 4000-deep list variant (0.9.2 died with a store page fault
# inside the recursive drop, stval just below the 0x20000000 heap base).
def build(n):
    a = []
    for _ in range(n):
        a = [a]
    return a

def gen(n):
    print("BUILD")
    a = build(n)
    print("BUILT")
    a = None
    print("DROPPED")
    yield 1

for _v in gen(4000):
    print("YIELD", _v)
print("FIBER DROP OK 4000")
