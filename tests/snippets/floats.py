import math

from testutils import assert_raises

1 + 1.1

a = 1.2
b = 1.3
c = 1.2
assert a < b
assert not b < a
assert a <= b
assert a <= c

assert b > a
assert not a > b
assert not a > c
assert b >= a
assert c >= a
assert not a >= b

assert a + b == 2.5
assert a - c == 0
assert a / c == 1

assert a < 5
assert a <= 5
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

# check that magic methods are implemented for ints and floats

assert 1.0.__add__(1.0) == 2.0
assert 1.0.__radd__(1.0) == 2.0
assert 2.0.__sub__(1.0) == 1.0
assert 2.0.__rmul__(1.0) == 2.0
assert 1.0.__truediv__(2.0) == 0.5
assert 1.0.__rtruediv__(2.0) == 2.0

assert 1.0.__add__(1) == 2.0
assert 1.0.__radd__(1) == 2.0
assert 2.0.__sub__(1) == 1.0
assert 2.0.__rmul__(1) == 2.0
assert 1.0.__truediv__(2) == 0.5
assert 1.0.__rtruediv__(2) == 2.0
assert 2.0.__mul__(1) == 2.0
assert 2.0.__rsub__(1) == -1.0

assert (1.7).real == 1.7
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

# Test special case for lexer, float starts with a dot:
a = .5
assert a == 0.5

