x = [1, 2, 3]
assert x[0] == 1
assert x[1] == 2
# assert x[7]

y = [2, *x]
assert y == [2, 1, 2, 3]

y.extend(x)
assert y == [2, 1, 2, 3, 1, 2, 3]

