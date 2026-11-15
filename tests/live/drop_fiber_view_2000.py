# F-r3-rt-1 coverage: a dict_items view is the last owner of a 2000-deep
# dict chain (v = a.items(); a = None); dropping the view frees the entries.
def build(n):
    a = None
    for _ in range(n):
        a = {"k": a}
    return a

def gen(n):
    a = build(n)
    v = a.items()
    print("VIEW BUILT")
    a = None
    v = None
    print("VIEW DROPPED")
    yield 1

for _v in gen(2000):
    pass
print("VIEW FIBER OK")
