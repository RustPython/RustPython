# int to int comparisons

assert 1 == 1
assert not 1 != 1

assert (1).__eq__(1)
assert not (1).__ne__(1)

# int to float comparisons

assert 1 == 1.0
assert not 1 != 1.0
assert not 1 > 1.0
assert not 1 < 1.0
assert 1 >= 1.0
assert 1 <= 1.0

# magic methods should only be implemented for other ints

assert (1).__eq__(1) == True
assert (1).__ne__(1) == False
assert (1).__gt__(1) == False
assert (1).__ge__(1) == True
assert (1).__lt__(1) == False
assert (1).__le__(1) == True
assert (1).__add__(1) == 2
assert (1).__radd__(1) == 2
assert (2).__sub__(1) == 1
assert (2).__rsub__(1) == -1
assert (2).__mul__(1) == 2
assert (2).__rmul__(1) == 2
assert (2).__truediv__(1) == 2.0
assert (2).__rtruediv__(1) == 0.5

assert (1).__eq__(1.0) == NotImplemented
assert (1).__ne__(1.0) == NotImplemented
assert (1).__gt__(1.0) == NotImplemented
assert (1).__ge__(1.0) == NotImplemented
assert (1).__lt__(1.0) == NotImplemented
assert (1).__le__(1.0) == NotImplemented
assert (1).__add__(1.0) == NotImplemented
assert (2).__sub__(1.0) == NotImplemented
assert (1).__radd__(1.0) == NotImplemented
assert (2).__rsub__(1.0) == NotImplemented
assert (2).__mul__(1.0) == NotImplemented
assert (2).__rmul__(1.0) == NotImplemented
assert (2).__truediv__(1.0) == NotImplemented
assert (2).__rtruediv__(1.0) == NotImplemented
