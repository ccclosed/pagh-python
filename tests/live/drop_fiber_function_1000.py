# F-r3-rt-1 coverage: a chain of 1000 closures, each capturing the previous
# function in its environment. Dropping the head walks Function -> Env -> Value
# links; the iterative teardown keeps that off the native stack. 1000 (not
# 4000) keeps the process inside the kernel's 1 MiB heap.
def make(prev):
    def f():
        return prev
    return f

def build(n):
    f = None
    for _ in range(n):
        f = make(f)
    return f

def gen(n):
    f = build(n)
    print("FUNCTION BUILT")
    f = None
    print("FUNCTION DROPPED")
    yield 1

for _v in gen(1000):
    pass
print("FUNCTION FIBER OK")
