from testutils import assertRaises

def dict_eq(d1, d2):
    return (all(k in d2 and d1[k] == d2[k] for k in d1)
            and all(k in d1 and d1[k] == d2[k] for k in d2))


assert dict_eq(dict(a=2, b=3), {'a': 2, 'b': 3})
assert dict_eq(dict({'a': 2, 'b': 3}, b=4), {'a': 2, 'b': 4})
assert dict_eq(dict([('a', 2), ('b', 3)]), {'a': 2, 'b': 3})

a = {'g': 5}
b = {'a': a, 'd': 9}
c = dict(b)
c['d'] = 3
c['a']['g'] = 2
assert dict_eq(a, {'g': 2})
assert dict_eq(b, {'a': a, 'd': 9})

a.clear()
assert len(a) == 0

a = {'a': 5, 'b': 6}
res = set()
for value in a.values():
        res.add(value)
assert res == set([5,6])

count = 0
for (key, value) in a.items():
        assert a[key] == value
        count += 1
assert count == len(a)

res = set()
for key in a.keys():
        res.add(key)
assert res == set(['a','b'])

x = {}
x[1] = 1
assert x[1] == 1


x[7] = 7
x[2] = 2
x[(5, 6)] = 5

with assertRaises(KeyError):
    x["not here"]

with assertRaises(TypeError):
    x[[]] # Unhashable type.

x["here"] = "here"
assert x.get("not here", "default") == "default"
assert x.get("here", "default") == "here"
assert x.get("not here") == None

class LengthDict(dict):
    def __getitem__(self, k):
        return len(k)

x = LengthDict()
assert type(x) == LengthDict
assert x['word'] == 4
assert x.get('word') is None

assert 5 == eval("a + word", LengthDict())


class Squares(dict):
    def __missing__(self, k):
        v = k * k
        self[k] = v
        return v

x = Squares()
assert x[-5] == 25

# An object that hashes to the same value always, and compares equal if any its values match.
class Hashable(object):
    def __init__(self, *args):
        self.values = args
    def __hash__(self):
        return 1
    def __eq__(self, other):
        for x in self.values:
            for y in other.values:
                if x == y:
                    return True
        return False

x = {}
x[Hashable(1,2)] = 8

assert x[Hashable(1,2)] == 8
assert x[Hashable(3,1)] == 8

x[Hashable(8)] = 19
x[Hashable(19,8)] = 1
assert x[Hashable(8)] == 1
assert len(x) == 2

assert list({'a': 2, 'b': 10}) == ['a', 'b']
x = {}
x['a'] = 2
x['b'] = 10
assert list(x) == ['a', 'b']
