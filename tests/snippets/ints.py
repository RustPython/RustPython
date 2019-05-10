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
assert (2).__pow__(3) == 8
assert (10).__pow__(-1) == 0.1
assert (2).__rpow__(3) == 9

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
assert (2).__pow__(3.0) == NotImplemented
assert (2).__rpow__(3.0) == NotImplemented

assert 10 // 4 == 2
assert -10 // 4 == -3
assert 10 // -4 == -3
assert -10 // -4 == 2

assert int() == 0
assert int("101", 2) == 5
assert int("101", base=2) == 5
assert int(1) == 1

assert int.from_bytes(b'\x00\x10', 'big') == 16
assert int.from_bytes(b'\x00\x10', 'little') == 4096
assert int.from_bytes(b'\xfc\x00', 'big', signed=True) == -1024
assert int.from_bytes(b'\xfc\x00', 'big', signed=False) == 64512

with assertRaises(TypeError):
    int(base=2)

with assertRaises(TypeError):
    int(1, base=2)

with assertRaises(TypeError):
    # check that first parameter is truly positional only
    int(val_options=1)

class A(object):
    def __int__(self):
        return 10

assert int(A()) == 10

class B(object):
    pass

b = B()
b.__int__ = lambda: 20

with assertRaises(TypeError):
    assert int(b) == 20

class C(object):
    def __int__(self):
        return 'str'

with assertRaises(TypeError):
    int(C())

class I(int):
    def __int__(self):
        return 3

assert int(I(1)) == 3

class F(float):
    def __int__(self):
        return 3

assert int(F(1.2)) == 3
