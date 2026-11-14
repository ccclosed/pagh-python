# F-r3-rt-1 coverage: the 4000-deep list is released when a function frame is
# torn down (release_env -> Env::clear_for_pool) on the generator fiber, not by
# an explicit assignment. Env teardown uses the same iterative worklist.
def build(n):
    a = []
    for _ in range(n):
        a = [a]
    return a

def holder(n):
    a = build(n)
    print("ENV BUILT")
    return 1

def gen(n):
    holder(n)
    print("ENV DROPPED")
    yield 1

for _v in gen(4000):
    pass
print("ENV FIBER OK")
