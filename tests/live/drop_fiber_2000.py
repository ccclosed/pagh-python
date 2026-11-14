# F-r3-rt-1 PoC: release a 2000-deep list Value chain from inside a generator.
# 0.9.2 charged AST nodes and checked fiber headroom before each node, but the
# Rc<RefCell<Vec<Value>>> teardown was recursive native drop glue that never
# went through eval_expr/exec_stmt: it overran the 64 KiB fiber (no guard page)
# and corrupted the interpreter heap. 0.9.3 drains every container iteratively.
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

for _v in gen(2000):
    print("YIELD", _v)
print("FIBER DROP OK 2000")
