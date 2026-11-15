# F-r3-rt-1 coverage: the iterator is the last owner of a 4000-deep list
# (iter(a); a = None), so dropping the iterator frees the container chain.
def build(n):
    a = []
    for _ in range(n):
        a = [a]
    return a

def gen(n):
    a = build(n)
    it = iter(a)
    print("ITERATOR BUILT")
    a = None
    it = None
    print("ITERATOR DROPPED")
    yield 1

for _v in gen(4000):
    pass
print("ITERATOR FIBER OK")
