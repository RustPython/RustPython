from testutils import assert_raises

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

assert (1).__eq__(1) is True
assert (1).__ne__(1) is False
assert (1).__gt__(1) is False
assert (1).__ge__(1) is True
assert (1).__lt__(1) is False
assert (1).__le__(1) is True
assert (1).__add__(1) == 2
assert (1).__radd__(1) == 2
assert (2).__sub__(1) == 1
assert (2).__rsub__(1) == -1
assert (2).__mul__(1) == 2
assert (2).__rmul__(1) == 2
assert (2).__truediv__(1) == 2.0
with assert_raises(ZeroDivisionError):
    (2).__truediv__(0)
assert (2).__rtruediv__(1) == 0.5
assert (-2).__floordiv__(3) == -1
with assert_raises(ZeroDivisionError):
    (2).__floordiv__(0)
assert (-3).__rfloordiv__(2) == -1
assert (-2).__divmod__(3) == (-1, 1)
with assert_raises(ZeroDivisionError):
    (2).__divmod__(0)
assert (-3).__rdivmod__(2) == (-1, -1)
assert (2).__pow__(3) == 8
assert (10).__pow__(-1) == 0.1
with assert_raises(ZeroDivisionError):
    (0).__pow__(-1)
assert (2).__rpow__(3) == 9
assert (10).__mod__(5) == 0
assert (10).__mod__(6) == 4
with assert_raises(ZeroDivisionError):
    (10).__mod__(0)
assert (5).__rmod__(10) == 0
assert (6).__rmod__(10) == 4
with assert_raises(ZeroDivisionError):
    (0).__rmod__(10)

# as_integer_ratio
# TODO uncomment the following tests once #1705 lands (or when the CPython version in test pipeline is upgraded to 3.8)
# assert (42).as_integer_ratio() == (42, 1)
# assert (-17).as_integer_ratio() == (-17, 1)
# assert (0).as_integer_ratio() == (0, 1)

# real/imag attributes
assert (1).real == 1
assert (1).imag == 0
# numerator/denominator attributes
assert (1).numerator == 1
assert (1).denominator == 1
assert (10).numerator == 10
assert (10).denominator == 1
assert (-10).numerator == -10
assert (-10).denominator == 1

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
assert (2).__mod__(3.0) == NotImplemented
assert (2).__rmod__(3.0) == NotImplemented

assert 10 // 4 == 2
assert -10 // 4 == -3
assert 10 // -4 == -3
assert -10 // -4 == 2

assert int() == 0
assert int(1) == 1
assert int("101", 2) == 5
assert int("101", base=2) == 5

# implied base
assert int('1', base=0) == 1
assert int('123', base=0) == 123
assert int('0b101', base=0) == 5
assert int('0B101', base=0) == 5
assert int('0o100', base=0) == 64
assert int('0O100', base=0) == 64
assert int('0xFF', base=0) == 255
assert int('0XFF', base=0) == 255
with assert_raises(ValueError):
    int('0xFF', base=10)
with assert_raises(ValueError):
    int('0oFF', base=10)
with assert_raises(ValueError):
    int('0bFF', base=10)
with assert_raises(ValueError):
    int('0bFF', base=10)
with assert_raises(ValueError):
    int(b"F\xc3\xb8\xc3\xb6\xbbB\xc3\xa5r")
with assert_raises(ValueError):
    int(b"F\xc3\xb8\xc3\xb6\xbbB\xc3\xa5r")

# string looks like radix
assert int('0b1', base=12) == 133
assert int('0o1', base=25) == 601
assert int('0x1', base=34) == 1123

# underscore
assert int('0xFF_FF_FF', base=16) == 16_777_215
with assert_raises(ValueError):
    int("_123_")
with assert_raises(ValueError):
    int("123_")
with assert_raises(ValueError):
    int("_123")
with assert_raises(ValueError):
    int("1__23")

assert int('0x_10', base=0) == 16

# signed
assert int('-123') == -123
assert int('+0b101', base=2) == +5

# trailing spaces
assert int(' 1') == 1
assert int('1 ') == 1
assert int(' 1 ') == 1
assert int('10', base=0) == 10

# type byte, signed, implied base
assert int(b'     -0XFF ', base=0) == -255

assert int.from_bytes(b'\x00\x10', 'big') == 16
assert int.from_bytes(b'\x00\x10', 'little') == 4096
assert int.from_bytes(b'\x00\x10', byteorder='big') == 16
assert int.from_bytes(b'\x00\x10', byteorder='little') == 4096
assert int.from_bytes(bytes=b'\x00\x10', byteorder='big') == 16
assert int.from_bytes(bytes=b'\x00\x10', byteorder='little') == 4096

assert int.from_bytes(b'\xfc\x00', 'big', signed=True) == -1024
assert int.from_bytes(b'\xfc\x00', 'big', signed=False) == 64512
assert int.from_bytes(b'\xfc\x00', byteorder='big', signed=True) == -1024
assert int.from_bytes(b'\xfc\x00', byteorder='big', signed=False) == 64512
assert int.from_bytes(bytes=b'\xfc\x00', byteorder='big', signed=True) == -1024
assert int.from_bytes(bytes=b'\xfc\x00', byteorder='big', signed=False) == 64512

assert int.from_bytes([255, 0, 0], 'big') == 16711680
assert int.from_bytes([255, 0, 0], 'little') == 255
assert int.from_bytes([255, 0, 0], 'big', signed=False) == 16711680
assert int.from_bytes([255, 0, 0], 'big', signed=True) == -65536

with assert_raises(ValueError):
    int.from_bytes(b'\x00\x10', 'something')

with assert_raises(ValueError):
    int.from_bytes([256, 0, 0], 'big')

with assert_raises(TypeError):
    int.from_bytes(['something', 0, 0], 'big')

assert (1024).to_bytes(4, 'big') == b'\x00\x00\x04\x00'
assert (1024).to_bytes(2, 'little') == b'\x00\x04'
assert (1024).to_bytes(4, byteorder='big') == b'\x00\x00\x04\x00'
assert (1024).to_bytes(2, byteorder='little') == b'\x00\x04'
assert (1024).to_bytes(length=4, byteorder='big') == b'\x00\x00\x04\x00'
assert (1024).to_bytes(length=2, byteorder='little') == b'\x00\x04'

assert (-1024).to_bytes(4, 'big', signed=True) == b'\xff\xff\xfc\x00'
assert (-1024).to_bytes(4, 'little', signed=True) == b'\x00\xfc\xff\xff'

assert (2147483647).to_bytes(8, 'big', signed=False) == b'\x00\x00\x00\x00\x7f\xff\xff\xff'
assert (-2147483648).to_bytes(8, 'little', signed=True) == b'\x00\x00\x00\x80\xff\xff\xff\xff'
assert (2147483647).to_bytes(8, byteorder='big', signed=False) == b'\x00\x00\x00\x00\x7f\xff\xff\xff'
assert (-2147483648).to_bytes(8, byteorder='little', signed=True) == b'\x00\x00\x00\x80\xff\xff\xff\xff'
assert (2147483647).to_bytes(length=8, byteorder='big', signed=False) == b'\x00\x00\x00\x00\x7f\xff\xff\xff'
assert (-2147483648).to_bytes(length=8, byteorder='little', signed=True) == b'\x00\x00\x00\x80\xff\xff\xff\xff'

with assert_raises(ValueError):
    (1024).to_bytes(4, 'something')

with assert_raises(OverflowError):
    (-1024).to_bytes(4, 'big')

with assert_raises(OverflowError):
    (1024).to_bytes(10000000000000000000000, 'big')

with assert_raises(OverflowError):
    (1024).to_bytes(1, 'big')

with assert_raises(ValueError):
    # check base first
    int(' 1 ', base=1)

with assert_raises(ValueError):
    int(' 1 ', base=37)

with assert_raises(ValueError):
    int(' 1 ', base=-1)

with assert_raises(ValueError):
    int(' 1 ', base=1000000000000000)

with assert_raises(ValueError):
    int(' 1 ', base=-1000000000000000)

with assert_raises(TypeError):
    int(base=2)

with assert_raises(TypeError):
    int(1, base=2)

with assert_raises(TypeError):
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

with assert_raises(TypeError):
    assert int(b) == 20

class C(object):
    def __int__(self):
        return 'str'

with assert_raises(TypeError):
    int(C())

class I(int):
    def __int__(self):
        return 3

assert int(I(1)) == 3

class F(float):
    def __int__(self):
        return 3

assert int(F(1.2)) == 3

class BadInt(int):
    def __int__(self):
        return 42.0

with assert_raises(TypeError):
    int(BadInt())

assert isinstance((0).__round__(), int)
assert isinstance((1).__round__(), int)
assert (0).__round__() == 0
assert (1).__round__() == 1
assert isinstance((0).__round__(0), int)
assert isinstance((1).__round__(0), int)
assert (0).__round__(0) == 0
assert (1).__round__(0) == 1
assert_raises(TypeError, lambda: (0).__round__(None))
assert_raises(TypeError, lambda: (1).__round__(None))
assert_raises(TypeError, lambda: (0).__round__(0.0))
assert_raises(TypeError, lambda: (1).__round__(0.0))

assert 00 == 0
assert 0_0 == 0
assert 03.2 == 3.2
assert 3+02j == 3+2j

# Invalid syntax:
src = """
b = 02
"""

with assert_raises(SyntaxError):
    exec(src)

# Invalid syntax:
src = """
b = 03 + 2j
"""

with assert_raises(SyntaxError):
    exec(src)

# Small int cache in [-5..256]
assert 1 is 1  # noqa
x = 6
assert 5 is (x-1)  # noqa
