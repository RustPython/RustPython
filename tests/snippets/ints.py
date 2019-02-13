assert not 1 > 1.0
assert not 1 < 1.0
assert 1 >= 1.0
assert 1 <= 1.0

assert (1).__gt__(1.0) == NotImplemented
assert (1).__ge__(1.0) == NotImplemented
assert (1).__lt__(1.0) == NotImplemented
assert (1).__le__(1.0) == NotImplemented
