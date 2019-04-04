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
