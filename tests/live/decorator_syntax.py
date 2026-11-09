# A decorator on a non-definition is a syntax error (checked live by the
# driver: the exit code must be 1 and the message must name the problem).
@double
x = 1
