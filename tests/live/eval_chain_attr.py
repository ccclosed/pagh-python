# Attribute chains are another production the parser folds in a loop:
# `c.self.self...` is one Attribute node per link and recurses in eval_expr.
# 300 links are past EXPR_RECURSION_LIMIT and raise RecursionError cleanly;
# no stack fault.
class C:
    pass

c = C()
c.self = c
x = c.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self.self
print('ATTR OK', x is c)
