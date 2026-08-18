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


# str and bytes comparisons, through a function so that the operands are not
# constants the compiler can fold, and in a loop so the specialized comparison
# is reached.
def cmp_all(a, b):
    return (a == b, a != b, a < b, a <= b, a > b, a >= b)


def check(a, b, expected):
    for _ in range(200):
        assert cmp_all(a, b) == expected, (a, b, cmp_all(a, b), expected)


EQ = (True, False, False, True, False, True)
LT = (False, True, True, True, False, False)
GT = (False, True, False, False, True, True)

same = "abc" * 3
check(same, same, EQ)  # the very same object
check(same, "abcabcabc", EQ)  # equal, distinct objects
check("abc", "abd", LT)  # same length, differing content
check("abc", "abcd", LT)  # a prefix is less than what extends it
check("abcd", "abc", GT)
check("", "a", LT)
check("", "", EQ)
check("\ud800", "\ud800", EQ)  # lone surrogates are compared as themselves
check("\ud800", "\udfff", LT)
check("a\U0001f600", "a\U0001f600", EQ)
check("가나다", "가나다", EQ)
check("가나", "가나다", LT)

# Comparing with a non-string is never an error for == and !=.
assert not "abc" == 3
assert "abc" != 3

bsame = b"abc" * 3
check(bsame, bsame, EQ)
check(bsame, b"abcabcabc", EQ)
check(b"abc", b"abd", LT)
check(b"abc", b"abcd", LT)
check(b"abcd", b"abc", GT)
check(bytearray(b"abc"), bytearray(b"abcd"), LT)
check(bytearray(b"abc"), b"abc", EQ)  # bytearray and bytes compare by content
check(b"abc", bytearray(b"abd"), LT)
assert not b"abc" == "abc"
assert b"abc" != "abc"
