import math

from testutils import assert_raises

1 + 1.1

a = 1.2
b = 1.3
c = 1.2
z = 2
ov = 10 ** 1000

assert -a == -1.2

assert a < b
assert not b < a
assert a <= b
assert a <= c
assert a < z

assert b > a
assert not a > b
assert not a > c
assert b >= a
assert c >= a
assert not a >= b
assert z > a

assert a + b == 2.5
assert a - c == 0
assert a / c == 1
assert a % c == 0
assert a + z == 3.2
assert z + a == 3.2
assert a - z == -0.8
assert z - a == 0.8
assert a / z == 0.6
assert 6 / a == 5.0
assert 2.0 % z == 0.0
assert z % 2.0 == 0.0
assert_raises(OverflowError, lambda: a + ov)
assert_raises(OverflowError, lambda: a - ov)
assert_raises(OverflowError, lambda: a * ov)
assert_raises(OverflowError, lambda: a / ov)
assert_raises(OverflowError, lambda: a // ov)
assert_raises(OverflowError, lambda: a % ov)
assert_raises(OverflowError, lambda: a ** ov)
assert_raises(OverflowError, lambda: ov + a)
assert_raises(OverflowError, lambda: ov - a)
assert_raises(OverflowError, lambda: ov * a)
assert_raises(OverflowError, lambda: ov / a)
assert_raises(OverflowError, lambda: ov // a)
assert_raises(OverflowError, lambda: ov % a)
# assert_raises(OverflowError, lambda: ov ** a)

assert a < 5
assert a <= 5
assert a < 5.5
assert a <= 5.5
try:
    assert a < 'a'
except TypeError:
    pass
try:
    assert a <= 'a'
except TypeError:
    pass
assert a > 1
assert a >= 1
try:
    assert a > 'a'
except TypeError:
    pass
try:
    assert a >= 'a'
except TypeError:
    pass

assert math.isnan(float('nan'))
assert math.isnan(float('NaN'))
assert math.isnan(float('+NaN'))
assert math.isnan(float('-NaN'))

assert math.isinf(float('inf'))
assert math.isinf(float('Inf'))
assert math.isinf(float('+Inf'))
assert math.isinf(float('-Inf'))

assert float('+Inf') > 0
assert float('-Inf') < 0

assert float('3.14') == 3.14
assert float('2.99e-23') == 2.99e-23

assert float(b'3.14') == 3.14
assert float(b'2.99e-23') == 2.99e-23

assert_raises(ValueError, lambda: float('foo'))
assert_raises(OverflowError, lambda: float(2**10000))

# check eq and hash for small numbers

assert 1.0 == 1
assert 1.0 == True
assert 0.0 == 0
assert 0.0 == False
assert hash(1.0) == hash(1)
assert hash(1.0) == hash(True)
assert hash(0.0) == hash(0)
assert hash(0.0) == hash(False)
assert hash(1.0) != hash(1.0000000001)

assert 5.0 in {3, 4, 5}
assert {-1: 2}[-1.0] == 2

# check that magic methods are implemented for ints and floats

assert 1.0.__add__(1.0) == 2.0
assert 1.0.__radd__(1.0) == 2.0
assert 2.0.__sub__(1.0) == 1.0
assert 2.0.__rmul__(1.0) == 2.0
assert 1.0.__truediv__(2.0) == 0.5
assert 1.0.__rtruediv__(2.0) == 2.0
assert 2.5.__divmod__(2.0) == (1.0, 0.5)
assert 2.0.__rdivmod__(2.5) == (1.0, 0.5)

assert 1.0.__add__(1) == 2.0
assert 1.0.__radd__(1) == 2.0
assert 2.0.__sub__(1) == 1.0
assert 2.0.__rmul__(1) == 2.0
assert 1.0.__truediv__(2) == 0.5
assert 1.0.__rtruediv__(2) == 2.0
assert 2.0.__mul__(1) == 2.0
assert 2.0.__rsub__(1) == -1.0
assert 2.0.__mod__(2) == 0.0
assert 2.0.__rmod__(2) == 0.0
assert_raises(ZeroDivisionError, lambda: 2.0 / 0)
assert_raises(ZeroDivisionError, lambda: 2.0 // 0)
assert_raises(ZeroDivisionError, lambda: 2.0 % 0)
assert_raises(ZeroDivisionError, lambda: divmod(2.0, 0))
assert_raises(ZeroDivisionError, lambda: 2 / 0.0)
assert_raises(ZeroDivisionError, lambda: 2 // 0.0)
assert_raises(ZeroDivisionError, lambda: 2 % 0.0)
# assert_raises(ZeroDivisionError, lambda: divmod(2, 0.0))

assert 1.2.__int__() == 1
assert 1.2.__float__() == 1.2
assert 1.2.__trunc__() == 1
assert int(1.2) == 1
assert float(1.2) == 1.2
assert math.trunc(1.2) == 1
assert_raises(OverflowError, float('inf').__trunc__)
assert_raises(ValueError, float('nan').__trunc__)
assert 0.5.__round__() == 0.0
assert 1.5.__round__() == 2.0
assert 0.5.__round__(0) == 0.0
assert 1.5.__round__(0) == 2.0
assert 0.5.__round__(None) == 0.0
assert 1.5.__round__(None) == 2.0
assert_raises(OverflowError, float('inf').__round__)
assert_raises(ValueError, float('nan').__round__)

assert 1.2 ** 2 == 1.44
assert_raises(OverflowError, lambda: 1.2 ** (10 ** 1000))
assert 3 ** 2.0 == 9.0

assert (1.7).real == 1.7
assert (1.7).imag == 0.0
assert (1.7).conjugate() == 1.7
assert (1.3).is_integer() == False
assert (1.0).is_integer()    == True

assert (0.875).as_integer_ratio() == (7, 8)
assert (-0.875).as_integer_ratio() == (-7, 8)
assert (0.0).as_integer_ratio() == (0, 1)
assert (11.5).as_integer_ratio() == (23, 2)
assert (0.0).as_integer_ratio() == (0, 1)
assert (2.5).as_integer_ratio() == (5, 2)
assert (0.5).as_integer_ratio() == (1, 2)
assert (2.1).as_integer_ratio() == (4728779608739021, 2251799813685248)
assert (-2.1).as_integer_ratio() == (-4728779608739021, 2251799813685248)
assert (-2100.0).as_integer_ratio() == (-2100, 1)
assert (2.220446049250313e-16).as_integer_ratio() == (1, 4503599627370496)
assert (1.7976931348623157e+308).as_integer_ratio() == (179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368, 1)
assert (2.2250738585072014e-308).as_integer_ratio() == (1, 44942328371557897693232629769725618340449424473557664318357520289433168951375240783177119330601884005280028469967848339414697442203604155623211857659868531094441973356216371319075554900311523529863270738021251442209537670585615720368478277635206809290837627671146574559986811484619929076208839082406056034304)

assert_raises(OverflowError, float('inf').as_integer_ratio)
assert_raises(OverflowError, float('-inf').as_integer_ratio)
assert_raises(ValueError, float('nan').as_integer_ratio)

assert str(1.0) == '1.0'
assert str(0.0) == '0.0'
assert str(1.123456789) == '1.123456789'

# Test special case for lexer, float starts with a dot:
a = .5
assert a == 0.5

assert float.fromhex('0x0.0p+0') == 0.0
assert float.fromhex('-0x0.0p+0') == -0.0
assert float.fromhex('0x1.000000p+0') == 1.0
assert float.fromhex('-0x1.800000p+0') == -1.5
assert float.fromhex('inf') == float('inf')
assert math.isnan(float.fromhex('nan'))

assert (0.0).hex() == '0x0.0p+0'
assert (-0.0).hex() == '-0x0.0p+0'
assert (1.0).hex() == '0x1.0000000000000p+0'
assert (-1.5).hex() == '-0x1.8000000000000p+0'
assert float('inf').hex() == 'inf'
assert float('-inf').hex() == '-inf'
assert float('nan').hex() == 'nan'

#for _ in range(10000):
#    f = random.random() * random.randint(0, 0x10000000000000000)
#    assert f == float.fromhex(f.hex())
