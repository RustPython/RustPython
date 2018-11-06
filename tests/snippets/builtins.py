
a = list(map(str, [1, 2, 3]))
assert a == ['1', '2', '3']

x = sum(map(int, a))
assert x == 6

assert callable(type)
# TODO:
# assert callable(callable)

assert list(enumerate(['a', 'b', 'c'])) == [(0, 'a'), (1, 'b'), (2, 'c')]

assert type(frozenset) is type

