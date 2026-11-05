try:
    raise ValueError("bad value")
except ValueError as error:
    print("caught ValueError:", error)

try:
    value = 1 // 0
except ZeroDivisionError:
    print("caught ZeroDivisionError")

def fail():
    raise RuntimeError("from fail")

try:
    fail()
except RuntimeError as error:
    print("propagated:", error)

try:
    print("try body")
except Exception:
    print("not reached")
finally:
    print("finally runs")

try:
    missing = {}
    missing["key"]
except KeyError as error:
    print("caught KeyError:", error)
