# F-r3-rt-1: same shape with tuples (Tuple(Rc<Vec<Value>>)); 0.9.2 died in the
# recursive drop before printing TUPLE DROPPED.
def build(n):
    a = ()
    for _ in range(n):
        a = (a,)
    return a

def gen(n):
    a = build(n)
    print("TUPLE BUILT")
    a = None
    print("TUPLE DROPPED")
    yield 1

for _v in gen(4000):
    pass
print("TUPLE FIBER OK")
