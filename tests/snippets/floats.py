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
