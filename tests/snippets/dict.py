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

# Deleted values are correctly skipped over:
x = {'a': 1, 'b': 2, 'c': 3, 'd': 3}
del x['c']
it = iter(x.items())
assert ('a', 1) == next(it)
assert ('b', 2) == next(it)
assert ('d', 3) == next(it)
with assertRaises(StopIteration):
    next(it)

# Iterating a dictionary is just its keys:
assert ['a', 'b', 'd'] == list(x)

# Iterating view captures dictionary when iterated.
data = {1: 2, 3: 4}
items = data.items()
assert list(items) == [(1, 2), (3, 4)]
data[5] = 6
assert list(items) == [(1, 2), (3, 4), (5, 6)]

# Values can be changed during iteration.
data = {1: 2, 3: 4}
items = iter(data.items())
assert (1, 2) == next(items)
data[3] = "changed"
assert (3, "changed") == next(items)

# View isn't itself an iterator.
with assertRaises(TypeError):
    next(data.keys())

assert len(data.keys()) == 2

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

y = x.copy()
x['c'] = 12
assert dict_eq(y, {'a': 2, 'b': 10})

y.update({'c': 19, "d": -1, 'b': 12})
assert dict_eq(y, {'a': 2, 'b': 12, 'c': 19, 'd': -1})

y.update(y)
assert dict_eq(y, {'a': 2, 'b': 12, 'c': 19, 'd': -1})  # hasn't changed
