x = [1, 2, 3]
assert x[0] == 1
assert x[1] == 2
# assert x[7]

y = [2, *x]
assert y == [2, 1, 2, 3]

y.extend(x)
assert y == [2, 1, 2, 3, 1, 2, 3]

assert x * 0 == [], "list __mul__ by 0 failed"
assert x * 2 == [1, 2, 3, 1, 2, 3], "list __mul__ by 2 failed"

assert ['a', 'b', 'c'].index('b') == 1
assert [5, 6, 7].index(7) == 2
try:
    ['a', 'b', 'c'].index('z')
except ValueError:
    pass
else:
    assert False, "ValueError was not raised"
