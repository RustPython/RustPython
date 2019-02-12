import math

def assert_raises(expr, exc_type):
    """
    Helper function to assert `expr` raises an exception of type `exc_type`
    Args:
        expr: Callable
        exec_type: Exception
    Returns:
        None
    Raises:
        Assertion error on failure
    """
    try:
        expr(None)
    except exc_type:
        assert True
    else:
        assert False

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

assert_raises(lambda _: float('foo'), ValueError)
assert_raises(lambda _: float(2**10000), OverflowError)
