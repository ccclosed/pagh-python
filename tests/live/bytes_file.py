path = "/mnt/bytes.bin"
data = b"\x00\x01pagh\xff"
with open(path, "wb") as out:
    count = out.write(data)
print("binary write:", count, "bytes")
assert count == len(data)

with open(path, "rb") as source:
    loaded = source.read()
print("binary read:", loaded, "len:", len(loaded))
assert loaded == data
assert loaded[2:6] == b"pagh"

with open(path, "rb") as source:
    whole = source.readline()
assert whole == data
print("binary file ok")
