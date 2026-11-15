# F-r4-rt-2: the same comparison depth cap on the 1 MiB process stack. On
# stock 0.9.3 1000 nested lists compared successfully here (the process stack
# was deep enough) and only the fiber died at 350+; now ordering comparisons
# stop at 256 levels on every stack, so this is a clean RecursionError, exit 1.
def deep(n):
    a = []
    for _ in range(n):
        a = [a]
    return a


a = deep(1000)
b = deep(1000)
print("BUILT 1000")
r = a < b
print("CMP", r, "SURVIVED 1000")