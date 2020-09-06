from testutils import assert_raises

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

d = {'a': 123, 'b': 456}
assert list(reversed(d)) == ['b', 'a']
assert list(reversed(d.keys())) == ['b', 'a']
assert list(reversed(d.values())) == [456, 123]
assert list(reversed(d.items())) == [('b', 456), ('a', 123)]
with assert_raises(StopIteration):
    dict_reversed = reversed(d)
    for _ in range(len(d) + 1):
        next(dict_reversed)
assert 'dict' in dict().__doc__

