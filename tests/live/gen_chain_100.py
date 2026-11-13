# Audit F-r2-py-2: the same left-deep expression inside a generator used to
# overrun the 64 KiB heap fiber stack (no guard page) and die with wild-pointer
# faults (stval=0x1/0x0/0x44800000008/0x1fffffe8). The evaluator now checks
# the fiber headroom at every node; the bomb is a catchable RecursionError and
# the interpreter keeps running.
def g():
    yield 1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1+1

try:
    next(g())
except RecursionError as error:
    assert 'generator stack exhausted' in str(error)
    print('GEN GUARD OK:', error)
else:
    print('GEN GUARD FAILED')
assert 1 + 1 == 2
print('ALIVE AFTER GEN BOMB')
