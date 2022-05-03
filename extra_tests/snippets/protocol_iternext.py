

ls = [1, 2, 3]

i = iter(ls)
assert i.__next__() == 1
assert i.__next__() == 2
assert next(i) == 3

assert next(i, 'w00t') == 'w00t'

s = '你好'
i = iter(s)
i.__setstate__(1)
assert i.__next__() == '好'
assert i.__reduce__()[2] == 2
