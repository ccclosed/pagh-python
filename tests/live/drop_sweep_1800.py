# F-r3-rt-1 control: 1800 nested lists were below the 0.9.2 fiber-stack
# threshold (the audit measured ~1900) and always survived. After the fix this
# is ordinary work for the iterative teardown.
def build(n):
    a = []
    for _ in range(n):
        a = [a]
    return a

def gen(n):
    a = build(n)
    print("BUILT")
    a = None
    print("DROPPED")
    yield 1

for _v in gen(1800):
    pass
print("SURVIVED 1800")
