from testutils import assert_raises

# __abs__

assert abs(complex(3, 4)) == 5
assert abs(complex(3, -4)) == 5
assert abs(complex(1.5, 2.5)) == 2.9154759474226504

# __eq__

assert complex(1, -1) == complex(1, -1)
assert complex(1, 0) == 1
assert 1 == complex(1, 0)
assert complex(1, 1) != 1
assert 1 != complex(1, 1)
assert complex(1, 0) == 1.0
assert 1.0 == complex(1, 0)
assert complex(1, 1) != 1.0
assert 1.0 != complex(1, 1)
assert complex(1, 0) != 1.5
assert not 1.0 != complex(1, 0)
assert bool(complex(1, 0))
assert complex(1, 2) != complex(1, 1)
assert complex(1, 2) != 'foo'
assert complex(1, 2).__eq__('foo') == NotImplemented
assert 1j != 10 ** 1000

# __mul__, __rmul__

assert complex(2, -3) * complex(-5, 7) == complex(11, 29)
assert complex(2, -3) * 5 == complex(10, -15)
assert 5 * complex(2, -3) == complex(2, -3) * 5

# __truediv__, __rtruediv__

assert complex(2, -3) / 2 == complex(1, -1.5)
assert 5 / complex(3, -4) == complex(0.6, 0.8)

# __mod__, __rmod__
# "can't mod complex numbers.
assert_raises(TypeError, lambda: complex(2, -3) % 2)
assert_raises(TypeError, lambda: 2 % complex(2, -3))

# __floordiv__, __rfloordiv__
# can't take floor of complex number.
assert_raises(TypeError, lambda: complex(2, -3) // 2)
assert_raises(TypeError, lambda: 2 // complex(2, -3))

# __divmod__, __rdivmod__
# "can't take floor or mod of complex number."
assert_raises(TypeError, lambda: divmod(complex(2, -3), 2))
assert_raises(TypeError, lambda: divmod(2, complex(2, -3)))

# __pow__, __rpow__

# assert 1j ** 2 == -1
assert complex(1) ** 2 == 1
assert 2 ** complex(2) == 4

# __pos__

assert +complex(0, 1) == complex(0, 1)
assert +complex(1, 0) == complex(1, 0)
assert +complex(1, -1) == complex(1, -1)
assert +complex(0, 0) == complex(0, 0)

# __neg__

assert -complex(1, -1) == complex(-1, 1)
assert -complex(0, 0) == complex(0, 0)

# __bool__

assert bool(complex(0, 0)) is False
assert bool(complex(0, 1)) is True
assert bool(complex(1, 0)) is True

# __hash__

assert hash(complex(1)) == hash(float(1)) == hash(int(1))
assert hash(complex(-1)) == hash(float(-1)) == hash(int(-1))
assert hash(complex(3.14)) == hash(float(3.14))
assert hash(complex(-float('inf'))) == hash(-float('inf'))
assert hash(1j) != hash(1)

# TODO: Find a way to test platform dependent values
assert hash(3.1 - 4.2j) == hash(3.1 - 4.2j)
assert hash(3.1 + 4.2j) == hash(3.1 + 4.2j)

# numbers.Complex

a = complex(3, 4)
b = 4j
assert a.real == 3
assert b.real == 0

assert a.imag == 4
assert b.imag == 4

assert a.conjugate() == 3 - 4j
assert b.conjugate() == -4j

# int and complex addition
assert 1 + 1j == complex(1, 1)
assert 1j + 1 == complex(1, 1)
assert (1j + 1) + 3 == complex(4, 1)
assert 3 + (1j + 1) == complex(4, 1)

# float and complex addition
assert 1.1 + 1.2j == complex(1.1, 1.2)
assert 1.3j + 1.4 == complex(1.4, 1.3)
assert (1.5j + 1.6) + 3 == complex(4.6, 1.5)
assert 3.5 + (1.1j + 1.2) == complex(4.7, 1.1)

# subtraction
assert 1 - 1j == complex(1, -1)
assert 1j - 1 == complex(-1, 1)
assert 2j - 1j == complex(0, 1)

# type error addition
assert_raises(TypeError, lambda: 1j + 'str')
assert_raises(TypeError, lambda: 1j - 'str')
assert_raises(TypeError, lambda: 'str' + 1j)
assert_raises(TypeError, lambda: 'str' - 1j)

# overflow
assert_raises(OverflowError, lambda: complex(10 ** 1000, 0))
assert_raises(OverflowError, lambda: complex(0, 10 ** 1000))
assert_raises(OverflowError, lambda: 0j + 10 ** 1000)

# str/repr
assert '(1+1j)' == str(1+1j)
assert '(1-1j)' == str(1-1j)
assert '(1+1j)' == repr(1+1j)
assert '(1-1j)' == repr(1-1j)

# __getnewargs__
assert (3 + 5j).__getnewargs__() == (3.0, 5.0)
assert (5j).__getnewargs__() == (0.0, 5.0)


class Complex():
    def __init__(self, real, imag):
        self.real = real
        self.imag = imag

    def __repr__(self):
        return "Com" + str((self.real, self.imag))

    def __sub__(self, other):
        return Complex(self.real - other, self.imag)

    def __rsub__(self, other):
        return Complex(other - self.real, -self.imag)

    def __eq__(self, other):
        return self.real == other.real and self.imag == other.imag

assert Complex(4, 5) - 3 == Complex(1, 5)
assert 7 - Complex(4, 5) == Complex(3, -5)

assert complex("5+2j") == 5 + 2j
assert complex("5-2j") == 5 - 2j
assert complex("-2j") == -2j
assert_raises(TypeError, lambda: complex("5+2j", 1))
assert_raises(ValueError, lambda: complex("abc"))

assert complex("1+10j") == 1+10j
assert complex(10) == 10+0j
assert complex(10.0) == 10+0j
assert complex(10) == 10+0j
assert complex(10+0j) == 10+0j
assert complex(1, 10) == 1+10j
assert complex(1, 10) == 1+10j
assert complex(1, 10.0) == 1+10j
assert complex(1, 10) == 1+10j
assert complex(1, 10) == 1+10j
assert complex(1, 10.0) == 1+10j
assert complex(1.0, 10) == 1+10j
assert complex(1.0, 10) == 1+10j
assert complex(1.0, 10.0) == 1+10j
assert complex(3.14+0j) == 3.14+0j
assert complex(3.14) == 3.14+0j
assert complex(314) == 314.0+0j
assert complex(314) == 314.0+0j
assert complex(3.14+0j, 0j) == 3.14+0j
assert complex(3.14, 0.0) == 3.14+0j
assert complex(314, 0) == 314.0+0j
assert complex(314, 0) == 314.0+0j
assert complex(0j, 3.14j) == -3.14+0j
assert complex(0.0, 3.14j) == -3.14+0j
assert complex(0j, 3.14) == 3.14j
assert complex(0.0, 3.14) == 3.14j
assert complex("1") == 1+0j
assert complex("1j") == 1j
assert complex() == 0
assert complex("-1") == -1
assert complex("+1") == +1
assert complex("(1+2j)") == 1+2j
assert complex("(1.3+2.2j)") == 1.3+2.2j
assert complex("3.14+1J") == 3.14+1j
assert complex(" ( +3.14-6J )") == 3.14-6j
assert complex(" ( +3.14-J )") == 3.14-1j
assert complex(" ( +3.14+j )") == 3.14+1j
assert complex("J") == 1j
assert complex("( j )") == 1j
assert complex("+J") == 1j
assert complex("( -j)") == -1j
assert complex('1e-500') == 0.0 + 0.0j
assert complex('-1e-500j') == 0.0 - 0.0j
assert complex('-1e-500+1e-500j') == -0.0 + 0.0j
