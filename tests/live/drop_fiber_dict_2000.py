# F-r3-rt-1 coverage: Dict(Rc<RefCell<Vec<(Value, Value)>>>) is the same
# recursive-drop shape; 0.9.3 drains keys and values through the worklist.
def build(n):
    a = None
    for _ in range(n):
        a = {"k": a}
    return a

def gen(n):
    a = build(n)
    print("DICT BUILT")
    a = None
    print("DICT DROPPED")
    yield 1

for _v in gen(2000):
    pass
print("DICT FIBER OK")
