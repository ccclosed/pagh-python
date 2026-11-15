# F-r3-rt-1 coverage: a 2000-link chain of Instance objects (attrs hold the
# next instance) released inside a generator.
class C:
    pass

def build(n):
    a = C()
    for _ in range(n):
        b = C()
        b.next = a
        a = b
    return a

def gen(n):
    a = build(n)
    print("INSTANCE BUILT")
    a = None
    print("INSTANCE DROPPED")
    yield 1

for _v in gen(2000):
    pass
print("INSTANCE FIBER OK")
