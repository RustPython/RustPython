from testutils import assert_raises, assertRaises

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

# check for argument handling

assert int("101", base=2) == 5

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

# real/imag attributes
assert (1).real == 1
assert (1).imag == 0

assert_raises(OverflowError, lambda: 1 << 10 ** 100000)

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


assert int() == 0
assert int("101", 2) == 5
assert int("101", base=2) == 5
assert int(1) == 1

with assertRaises(TypeError):
    int(base=2)

with assertRaises(TypeError):
    int(1, base=2)

with assertRaises(TypeError):
    # check that first parameter is truly positional only
    int(val_options=1)
