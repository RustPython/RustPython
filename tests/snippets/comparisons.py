
assert 1 < 2
assert 1 < 2 < 3
assert 5 == 5 == 5
assert (5 == 5) == True
assert 5 == 5 != 4 == 4 > 3 > 2 < 3 <= 3 != 0 == 0

assert not 1 > 2
assert not 5 == 5 == True
assert not 5 == 5 != 5 == 5
assert not 1 < 2 < 3 > 4
assert not 1 < 2 > 3 < 4
assert not 1 > 2 < 3 < 4
