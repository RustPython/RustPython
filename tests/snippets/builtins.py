x = sum(map(int, ['1', '2', '3']))
assert x == 6

assert callable(type)
# TODO:
# assert callable(callable)

assert type(frozenset) is type

assert 3 == eval('1+2')

code = compile('5+3', 'x.py', 'eval')
assert eval(code) == 8
