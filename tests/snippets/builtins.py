
a = list(map(str, [1, 2, 3]))
assert a == ['1', '2', '3']

x = sum(map(int, a))
assert x == 6

assert callable(type)
# TODO:
# assert callable(callable)

assert list(enumerate(['a', 'b', 'c'])) == [(0, 'a'), (1, 'b'), (2, 'c')]

assert type(frozenset) is type

assert list(zip(['a', 'b', 'c'], range(3), [9, 8, 7, 99])) == [('a', 0, 9), ('b', 1, 8), ('c', 2, 7)]

assert list(filter(lambda x: ((x % 2) == 0), [0, 1, 2])) == [0, 2]

