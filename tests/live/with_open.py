path = "/mnt/with.txt"
with open(path, "w") as out:
    out.write("alpha\nbeta\n")
with open(path) as source:
    text = source.read()
print("with file:", text.replace("\n", "|"))
assert text == "alpha\nbeta\n"
assert source.closed

class Trace:
    def __init__(self, name):
        self.name = name
        self.events = []

    def __enter__(self):
        print("enter", self.name)
        return self.name

    def __exit__(self, kind, value, tb):
        name = kind.__name__ if kind else None
        text = str(value) if value else None
        self.events.append((name, text))
        print("exit", self.name, self.events[-1])
        return False

first = Trace("first")
second = Trace("second")
with first as a, second as b:
    print("body", a, b)
print("events:", first.events, second.events)
assert first.events == [(None, None)]
assert second.events == [(None, None)]

class Suppress:
    def __enter__(self):
        return self

    def __exit__(self, kind, value, tb):
        self.seen = (kind.__name__, str(value))
        return True

with Suppress() as catcher:
    raise ValueError("suppressed")
print("suppressed:", catcher.seen)
assert catcher.seen == ("ValueError", "suppressed")

try:
    with open(path, "w") as out:
        raise RuntimeError("not suppressed")
except RuntimeError as error:
    print("file exit propagates:", error)
print("with ok")
