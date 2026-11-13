# Python call recursion regression (RECURSION_LIMIT = 56): 55 frames work on
# pagh-riscv's 1 MiB process stack, the 56th nested call raises RecursionError
# and the program can continue. 0.9.2 does not change this path; it makes sure
# the new evaluator guard does not eat the call budget.
def r(n):
    if n == 0:
        return 0
    return r(n - 1) + 1

assert r(55) == 55
try:
    r(60)
except RecursionError as error:
    assert 'maximum recursion depth exceeded' in str(error)
    print('CALL LIMIT CAUGHT:', error)
else:
    print('CALL LIMIT FAILED')
assert 6 * 7 == 42
print('RECURSION LIMIT OK')
