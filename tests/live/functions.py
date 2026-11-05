def fact(n):
    if n <= 1:
        return 1
    return n * fact(n - 1)

def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

def apply_twice(function, value):
    return function(function(value))

print("fact(6) =", fact(6))
print("fib(10) =", fib(10))
def add_three(value):
    return value + 3

print("nested calls:", apply_twice(add_three, 10))
