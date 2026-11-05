raw = b"pagh"
print("literal:", raw, "len:", len(raw), "index:", raw[0], "slice:", raw[1:3])
assert raw == b"pagh"
assert raw[0] == 112
assert raw[1:3] == b"ag"
assert b"ag" in raw
assert b"zz" not in raw

escaped = b"line\n\x41\xff"
print("escaped:", escaped)
assert len(escaped) == 7
assert escaped[5] == 65
assert escaped[6] == 255

text = "hello"
encoded = text.encode("utf-8")
decoded = encoded.decode("utf-8")
print("encode/decode:", encoded, decoded)
assert decoded == text
assert encoded == b"hello"
assert "héllo".encode("utf-8") == b"h\xc3\xa9llo"
assert b"h\xc3\xa9llo".decode("utf-8") == "héllo"

built = bytes([65, 66, 67])
zeros = bytes(2)
print("bytes():", built, zeros)
assert built == b"ABC"
assert zeros == b"\x00\x00"

assert b"abc" < b"abd"
assert b"abc" == b"abc"
assert b"abc" != b"abd"
assert list(b"AB") == [65, 66]
print("hex:", built.hex())
assert built.hex() == "414243"
print("bytes ok")
