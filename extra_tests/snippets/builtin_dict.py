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

d = {'a': 123, 'b': 456}
assert 1 not in d.items()
assert 'a' not in d.items()
assert 'a', 123 not in d.items()
assert () not in d.items()
assert (1) not in d.items()
assert ('a') not in d.items()
assert ('a', 123) in d.items()
assert ('b', 456) in d.items()
assert ('a', 123, 3) not in d.items()
assert ('a', 123, 'b', 456) not in d.items()

d = {1: 10, "a": "ABC", (3,4): 5}
assert 1 in d.keys()
assert (1) in d.keys()
assert "a" in d.keys()
assert (3,4) in d.keys()
assert () not in d.keys()
assert 10 not in d.keys()
assert (1, 10) not in d.keys()
assert "abc" not in d.keys()
assert ((3,4),5) not in d.keys()