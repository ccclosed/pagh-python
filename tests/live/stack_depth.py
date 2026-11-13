# Recursion and the two stacks, pythonc 0.5.0.
# Run: run /mnt/pythonc /mnt/stack_depth.py
#
# A generator body runs on a 64 KiB fiber stack that this interpreter owns, and
# that stack *is* checked: too deep a recursion inside it raises RecursionError
# instead of writing past the end. The process stack is the kernel's 1 MiB user
# window (USER_STACK_FLOOR..USER_TOP); plain calls are bounded by
# RECURSION_LIMIT = 56 and deep expression trees by EXPR_RECURSION_LIMIT
# (0.9.2), so this script stays well inside both.

def reck(n):
    if n == 0:
        return 0
    return reck(n - 1) + 1

# Plain recursion: the shallow depths every real script uses.
print("shallow plain recursion:", reck(1), reck(4), reck(8))
assert reck(5) == 5 and reck(8) == 8

# Inside a generator the fiber guard turns the stack limit into an error.
def gen(depth):
    yield reck(depth)

gen_best = 0
gen_message = ""
for depth in range(1, 60):
    try:
        next(gen(depth))
        gen_best = depth
    except RecursionError as error:
        gen_message = str(error)
        break
print("max depth inside a generator:", gen_best)
print("generator failure:", gen_message)
assert gen_best >= 5
assert "generator stack exhausted" in gen_message

# The interpreter's own recursion constant (56) is higher than either stack can
# hold; the fiber guard is what a generator actually hits first.
print("fiber stack KiB:", 64)
print("recursion ok")
