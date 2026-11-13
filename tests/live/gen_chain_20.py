# A short chain inside a generator still works: the fiber guard only refuses
# when the 64 KiB heap stack is nearly exhausted.
def g():
    yield 1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1

v = next(g())
assert v == 20
print('GEN CHAIN 20 OK', v)
