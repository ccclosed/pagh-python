# F-r4-rt-2 control: 250 nested sequences are inside the comparison depth
# limit. On stock 0.9.3 this compared fine (the recursive walk still fitted the
# 64 KiB generator fiber) while 350+ corrupted the heap.
def deep(n):
    a = []
    for _ in range(n):
        a = [a]
    return a


def deep_tuple(n):
    a = ()
    for _ in range(n):
        a = (a,)
    return a


def deep_leaf(n, leaf):
    a = [leaf]
    for _ in range(n):
        a = [a]
    return a


def list_compare(n):
    a = deep(n)
    b = deep(n)
    yield a < b


def tuple_compare(n):
    a = deep_tuple(n)
    b = deep_tuple(n)
    yield a < b


# Equal deep values compare Equal (False for <); the ordering is still found
# when the difference sits below the accepted nesting.
assert list(list_compare(250)) == [False]
assert list(tuple_compare(250)) == [False]
assert deep_leaf(249, [0]) < deep_leaf(249, [1])
print("CMP DEPTH 250 OK")
print("CMP TUPLE 250 OK")

# A descending list of shallow lists is insertion sort's worst case: thousands
# of comparisons of list pairs. The frame buffer is reused, so this must not
# grow the 1 MiB bump heap (a per-comparison allocation would OOM here).
shallow = [[i] for i in range(120, 0, -1)]
assert sorted(shallow) == [[i] for i in range(1, 121)]
print("CMP SORT SHALLOW OK")