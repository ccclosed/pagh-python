# F-r3-rt-1 coverage: Set(Rc<RefCell<Vec<Value>>>) holding a 2000-deep tuple.
# Building the set also exercises tuple key hashing, which is iterative in
# 0.9.3 for the same 64 KiB fiber reason.
def build(n):
    a = ()
    for _ in range(n):
        a = (a,)
    return a

def gen(n):
    a = build(n)
    s = {a}
    print("SET BUILT")
    s = None
    a = None
    print("SET DROPPED")
    yield 1

for _v in gen(2000):
    pass
print("SET FIBER OK")
