assert len(dict()) == 0

assert len({}) == 0
assert len({"a": "b"}) == 1
assert len({"a": "b", "b": 1}) == 2
assert len({"a": "b", "b": 1, "a" + "b": 2*2}) == 3

d = {}
d['a'] = d
assert repr(d) == "{'a': {...}}"

assert {'a': 123}.get('a') == 123
assert {'a': 123}.get('b') == None
assert {'a': 123}.get('b', 456) == 456
