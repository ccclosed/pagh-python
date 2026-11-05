import json

value = {
    "name": "pagh",
    "ok": True,
    "count": 3,
    "ratio": 1.5,
    "items": [1, 2, None, "x"],
}
text = json.dumps(value)
print("dumps:", text)
assert text == '{"name": "pagh", "ok": true, "count": 3, "ratio": 1.5, "items": [1, 2, null, "x"]}'
back = json.loads(text)
print("loads:", back)
assert back == value

assert json.loads('"a\\nb"') == "a\nb"
assert json.loads("[true, false, null]") == [True, False, None]
assert json.loads('{"n": -2.5}') == {"n": -2.5}

escaped = json.dumps({"quote": 'he said "hi"', "newline": "a\nb"})
print("escaped:", escaped)
assert escaped == '{"quote": "he said \\"hi\\"", "newline": "a\\nb"}'

try:
    json.loads("{bad}")
    parsed_bad = True
except ValueError as error:
    parsed_bad = False
assert not parsed_bad
print("json ok")
