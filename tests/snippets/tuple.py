assert (1,2) == (1,2)

x = (1,2)
assert x[0] == 1

y = (1,)
assert y[0] == 1

assert x * 3 == (1, 2, 1, 2, 1, 2)
# assert 3 * x == (1, 2, 1, 2, 1, 2)
assert x * 0 == ()
assert x * -1 == ()  # integers less than zero treated as 0
