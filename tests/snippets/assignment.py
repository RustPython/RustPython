x = 1
assert x == 1

x = 1, 2, 3
assert x == (1, 2, 3)

x, y = 1, 2
assert x == 1
assert y == 2

x, y = (y, x)

assert x == 2
assert y == 1

((x, y), z) = ((1, 2), 3)

assert (x, y, z) == (1, 2, 3)

q = (1, 2, 3)
(x, y, z) = q
assert y == q[1]

x = (a, b, c) = y = q

assert (a, b, c) == q
assert x == q
assert y == q
