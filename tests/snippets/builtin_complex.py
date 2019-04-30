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

assert_raises(
    TypeError,
    lambda: complex(2, -3) % 2,
    "can't mod complex numbers.")
assert_raises(
    TypeError,
    lambda: 2 % complex(2, -3),
    "can't mod complex numbers.")

# __floordiv__, __rfloordiv__

assert_raises(
    TypeError,
    lambda: complex(2, -3) // 2,
    "can't take floor of complex number.")
assert_raises(
    TypeError,
    lambda: 2 // complex(2, -3),
    "can't take floor of complex number.")

# __divmod__, __rdivmod__

assert_raises(
    TypeError,
    lambda: divmod(complex(2, -3), 2),
    "can't take floor or mod of complex number.")
assert_raises(
    TypeError,
    lambda: divmod(2, complex(2, -3)),
    "can't take floor or mod of complex number.")

# __pow__, __rpow__

# assert 1j ** 2 == -1
assert complex(1) ** 2 == 1
assert 2 ** complex(2) == 4

# __neg__

assert -complex(1, -1) == complex(-1, 1)
assert -complex(0, 0) == complex(0, 0)

# __bool__

assert bool(complex(0, 0)) is False
assert bool(complex(0, 1)) is True
assert bool(complex(1, 0)) is True

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
msg = 'int too large to convert to float'
assert_raises(OverflowError, lambda: complex(10 ** 1000, 0), msg)
assert_raises(OverflowError, lambda: complex(0, 10 ** 1000), msg)
assert_raises(OverflowError, lambda: 0j + 10 ** 1000, msg)
