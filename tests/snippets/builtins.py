x = sum(map(int, ['1', '2', '3']))
assert x == 6

assert callable(type)
# TODO:
# assert callable(callable)

assert list(enumerate(['a', 'b', 'c'])) == [(0, 'a'), (1, 'b'), (2, 'c')]

assert type(frozenset) is type

assert list(zip(['a', 'b', 'c'], range(3), [9, 8, 7, 99])) == [('a', 0, 9), ('b', 1, 8), ('c', 2, 7)]

assert 3 == eval('1+2')

code = compile('5+3', 'x.py', 'eval')
assert eval(code) == 8
