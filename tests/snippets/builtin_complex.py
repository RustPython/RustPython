from testutils import assertRaises

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

# __mul__

assert complex(2, -3) * complex(-5, 7) == complex(-21, 29)
assert complex(2, -3) * 5 == complex(10, -15)

# __neg__

assert -complex(1, -1) == complex(-1, 1)
assert -complex(0, 0) == complex(0, 0)

# __bool__

assert bool(complex(0, 0)) is False
assert bool(complex(0, 1)) is True
assert bool(complex(1, 0)) is True

# real

a = complex(3, 4)
b = 4j
assert a.real == 3
assert b.real == 0

# imag

assert a.imag == 4
assert b.imag == 4

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
with assertRaises(TypeError):
    assert 1j + 'str'
with assertRaises(TypeError):
    assert 1j - 'str'
with assertRaises(TypeError):
    assert 'str' + 1j
with assertRaises(TypeError):
    assert 'str' - 1j

# overflow
with assertRaises(OverflowError):
    complex(10 ** 1000, 0)
with assertRaises(OverflowError):
    complex(0, 10 ** 1000)
with assertRaises(OverflowError):
    complex(0, 0) + 10 ** 1000
