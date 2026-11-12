# `yield from` outside a function is rejected by the parser, not at run time.
items = [1, 2]
yield from items
