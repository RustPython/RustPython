from testutils import assert_raises

assert 1 < 2
assert 1 < 2 < 3
assert 5 == 5 == 5
assert (5 == 5) == True
assert 5 == 5 != 4 == 4 > 3 > 2 < 3 <= 3 != 0 == 0

assert not 1 > 2
assert not 5 == 5 == True
assert not 5 == 5 != 5 == 5
assert not 1 < 2 < 3 > 4
assert not 1 < 2 > 3 < 4
assert not 1 > 2 < 3 < 4


def test_type_error(x, y):
    assert_raises(TypeError, lambda: x < y)
    assert_raises(TypeError, lambda: x <= y)
    assert_raises(TypeError, lambda: x > y)
    assert_raises(TypeError, lambda: x >= y)


test_type_error([], 0)
test_type_error((), 0)


# 10**308 cannot be represented exactly in f64, thus it is not equal to 1e308 float
assert not (10**308 == 1e308)
# but the 1e308 float can be converted to big int and then it still should be equal to itself
assert int(1e308) == 1e308

# and the equalities should be the same when operands switch sides
assert not (1e308 == 10**308)
assert 1e308 == int(1e308)

# floats that cannot be converted to big ints shouldn’t crash the vm
import math

assert not (10**500 == math.inf)
assert not (math.inf == 10**500)
assert not (10**500 == math.nan)
assert not (math.nan == 10**500)

# comparisons
# floats with worse than integer precision
assert 2.0**54 > 2**54 - 1
assert 2.0**54 < 2**54 + 1
assert 2.0**54 >= 2**54 - 1
assert 2.0**54 <= 2**54 + 1
assert 2.0**54 == 2**54
assert not 2.0**54 == 2**54 + 1

# inverse operands
assert 2**54 - 1 < 2.0**54
assert 2**54 + 1 > 2.0**54
assert 2**54 - 1 <= 2.0**54
assert 2**54 + 1 >= 2.0**54
assert 2**54 == 2.0**54
assert not 2**54 + 1 == 2.0**54

assert not 2.0**54 < 2**54 - 1
assert not 2.0**54 > 2**54 + 1

# sub-int numbers
assert 1.3 > 1
assert 1.3 >= 1
assert 2.5 > 2
assert 2.5 >= 2
assert -0.3 < 0
assert -0.3 <= 0

# int out of float range comparisons
assert 10**500 > 2.0**54
assert -(10**500) < -0.12

# infinity and NaN comparisons
assert math.inf > 10**500
assert math.inf >= 10**500
assert not math.inf < 10**500

assert -math.inf < -10 * 500
assert -math.inf <= -10 * 500
assert not -math.inf > -10 * 500

assert not math.nan > 123
assert not math.nan < 123
assert not math.nan >= 123
assert not math.nan <= 123
