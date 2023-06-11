import math
from testutils import assert_raises, skip_if_unsupported

NAN = float('nan')
INF = float('inf')
NINF = float('-inf')

# assert(math.exp(2) == math.exp(2.0))
# assert(math.exp(True) == math.exp(1.0))
#
# class Conversible():
#     def __float__(self):
#         print("Converting to float now!")
#         return 1.1111
#
# assert math.log(1.1111) == math.log(Conversible())

# roundings
assert int.__trunc__
assert int.__floor__
assert int.__ceil__

# assert float.__trunc__


def float_floor_exists():
    assert float.__floor__


def float_ceil_exists():
    assert float.__ceil__


skip_if_unsupported(3, 9, float_floor_exists)
skip_if_unsupported(3, 9, float_ceil_exists)

assert math.trunc(2) == 2
assert math.ceil(3) == 3
assert math.floor(4) == 4

assert math.trunc(2.2) == 2
assert math.ceil(3.3) == 4
assert math.floor(4.4) == 4

assert isinstance(math.trunc(2.2), int)
assert isinstance(math.ceil(3.3), int)
assert isinstance(math.floor(4.4), int)

class A(object):
    def __trunc__(self):
        return 2

    def __ceil__(self):
        return 3

    def __floor__(self):
        return 4

assert math.trunc(A()) == 2
assert math.ceil(A()) == 3
assert math.floor(A()) == 4

class A(object):
    def __trunc__(self):
        return 2.2

    def __ceil__(self):
        return 3.3

    def __floor__(self):
        return 4.4

assert math.trunc(A()) == 2.2
assert math.ceil(A()) == 3.3
assert math.floor(A()) == 4.4

class A(object):
    def __trunc__(self):
        return 'trunc'

    def __ceil__(self):
        return 'ceil'

    def __floor__(self):
        return 'floor'

assert math.trunc(A()) == 'trunc'
assert math.ceil(A()) == 'ceil'
assert math.floor(A()) == 'floor'

with assert_raises(TypeError):
    math.trunc(object())
with assert_raises(TypeError):
    math.ceil(object())
with assert_raises(TypeError):
    math.floor(object())

isclose = math.isclose

def assertIsClose(a, b, *args, **kwargs):
    assert isclose(a, b, *args, **kwargs) == True, "%s and %s should be close!" % (a, b)

def assertIsNotClose(a, b, *args, **kwargs):
    assert isclose(a, b, *args, **kwargs) == False, "%s and %s should not be close!" % (a, b)

def assertAllClose(examples, *args, **kwargs):
    for a, b in examples:
        assertIsClose(a, b, *args, **kwargs)

def assertAllNotClose(examples, *args, **kwargs):
    for a, b in examples:
        assertIsNotClose(a, b, *args, **kwargs)

# test_negative_tolerances: ValueError should be raised if either tolerance is less than zero
assert_raises(ValueError, lambda: isclose(1, 1, rel_tol=-1e-100))
assert_raises(ValueError, lambda: isclose(1, 1, rel_tol=1e-100, abs_tol=-1e10))

# test_identical: identical values must test as close
identical_examples = [(2.0, 2.0),
                        (0.1e200, 0.1e200),
                        (1.123e-300, 1.123e-300),
                        (12345, 12345.0),
                        (0.0, -0.0),
                        (345678, 345678)]
assertAllClose(identical_examples, rel_tol=0.0, abs_tol=0.0)

# test_eight_decimal_places: examples that are close to 1e-8, but not 1e-9
eight_decimal_places_examples = [(1e8, 1e8 + 1),
                                 (-1e-8, -1.000000009e-8),
                                 (1.12345678, 1.12345679)]
assertAllClose(eight_decimal_places_examples, rel_tol=1e-08)
assertAllNotClose(eight_decimal_places_examples, rel_tol=1e-09)

# test_near_zero: values close to zero
near_zero_examples = [(1e-9, 0.0),
                      (-1e-9, 0.0),
                      (-1e-150, 0.0)]
# these should not be close to any rel_tol
assertAllNotClose(near_zero_examples, rel_tol=0.9)
# these should be close to abs_tol=1e-8
assertAllClose(near_zero_examples, abs_tol=1e-8)

# test_identical_infinite: these are close regardless of tolerance -- i.e. they are equal
assertIsClose(INF, INF)
assertIsClose(INF, INF, abs_tol=0.0)
assertIsClose(NINF, NINF)
assertIsClose(NINF, NINF, abs_tol=0.0)

# test_inf_ninf_nan(self): these should never be close (following IEEE 754 rules for equality)
not_close_examples = [(NAN, NAN),
                      (NAN, 1e-100),
                      (1e-100, NAN),
                      (INF, NAN),
                      (NAN, INF),
                      (INF, NINF),
                      (INF, 1.0),
                      (1.0, INF),
                      (INF, 1e308),
                      (1e308, INF)]
# use largest reasonable tolerance
assertAllNotClose(not_close_examples, abs_tol=0.999999999999999)

# test_zero_tolerance: test with zero tolerance
zero_tolerance_close_examples = [(1.0, 1.0),
                                 (-3.4, -3.4),
                                 (-1e-300, -1e-300)]
assertAllClose(zero_tolerance_close_examples, rel_tol=0.0)
zero_tolerance_not_close_examples = [(1.0, 1.000000000000001),
                                     (0.99999999999999, 1.0),
                                     (1.0e200, .999999999999999e200)]
assertAllNotClose(zero_tolerance_not_close_examples, rel_tol=0.0)

# test_asymmetry: test the asymmetry example from PEP 485
assertAllClose([(9, 10), (10, 9)], rel_tol=0.1)

# test_integers: test with integer values
integer_examples = [(100000001, 100000000),
                    (123456789, 123456788)]

assertAllClose(integer_examples, rel_tol=1e-8)
assertAllNotClose(integer_examples, rel_tol=1e-9)

# test_decimals: test with Decimal values
# test_fractions: test with Fraction values

assert math.copysign(1, 42) == 1.0
assert math.copysign(0., 42) == 0.0
assert math.copysign(1., -42) == -1.0
assert math.copysign(3, 0.) == 3.0
assert math.copysign(4., -0.) == -4.0
assert_raises(TypeError, math.copysign)
# copysign should let us distinguish signs of zeros
assert math.copysign(1., 0.) == 1.
assert math.copysign(1., -0.) == -1.
assert math.copysign(INF, 0.) == INF
assert math.copysign(INF, -0.) == NINF
assert math.copysign(NINF, 0.) == INF
assert math.copysign(NINF, -0.) == NINF
# and of infinities
assert math.copysign(1., INF) == 1.
assert math.copysign(1., NINF) == -1.
assert math.copysign(INF, INF) == INF
assert math.copysign(INF, NINF) == NINF
assert math.copysign(NINF, INF) == INF
assert math.copysign(NINF, NINF) == NINF
assert math.isnan(math.copysign(NAN, 1.))
assert math.isnan(math.copysign(NAN, INF))
assert math.isnan(math.copysign(NAN, NINF))
assert math.isnan(math.copysign(NAN, NAN))
# copysign(INF, NAN) may be INF or it may be NINF, since
# we don't know whether the sign bit of NAN is set on any
# given platform.
assert math.isinf(math.copysign(INF, NAN))
# similarly, copysign(2., NAN) could be 2. or -2.
assert abs(math.copysign(2., NAN)) == 2.

assert str(math.frexp(0.0)) == str((+0.0, 0))
assert str(math.frexp(-0.0)) == str((-0.0, 0))
assert math.frexp(1) == (0.5, 1)
assert math.frexp(1.5) == (0.75, 1)
assert_raises(TypeError, lambda: math.frexp(None))

assert str(math.ldexp(+0.0, 0)) == str(0.0)
assert str(math.ldexp(-0.0, 0)) == str(-0.0)
assert math.ldexp(0.5, 1) == 1
assert math.ldexp(0.75, 1) == 1.5
assert_raises(TypeError, lambda: math.ldexp(None, None))

assert math.frexp(INF) == (INF, 0)
assert str(math.frexp(NAN)) == str((NAN, 0))
assert_raises(TypeError, lambda: math.frexp(None))

assert math.gcd(0, 0) == 0
assert math.gcd(1, 0) == 1
assert math.gcd(0, 1) == 1
assert math.gcd(1, 1) == 1
assert math.gcd(-1, 1) == 1
assert math.gcd(1, -1) == 1
assert math.gcd(-1, -1) == 1
assert math.gcd(125, -255) == 5
assert_raises(TypeError, lambda: math.gcd(1.1, 2))

assert math.factorial(0) == 1
assert math.factorial(1) == 1
assert math.factorial(2) == 2
assert math.factorial(3) == 6
assert math.factorial(10) == 3628800
assert math.factorial(20) == 2432902008176640000
assert_raises(ValueError, lambda: math.factorial(-1))

if hasattr(math, 'nextafter'):
    try:
        assert math.nextafter(4503599627370496.0, -INF) == 4503599627370495.5
        assert math.nextafter(4503599627370496.0, INF) == 4503599627370497.0
        assert math.nextafter(9223372036854775808.0, 0.0) == 9223372036854774784.0
        assert math.nextafter(-9223372036854775808.0, 0.0) == -9223372036854774784.0
        assert math.nextafter(4503599627370496, -INF) == 4503599627370495.5
        assert math.nextafter(2.0, 2.0) == 2.0
        assert math.isnan(math.nextafter(NAN, 1.0))
    except NotImplementedError:
        # WASM
        pass

assert math.modf(1.25) == (0.25, 1.0)
assert math.modf(-1.25) == (-0.25, -1.0)
assert math.modf(2.56) == (0.56, 2.0)
assert math.modf(-2.56) == (-0.56, -2.0)
assert math.modf(1) == (0.0, 1.0)
assert math.modf(INF) == (0.0, INF)
assert math.modf(NINF) == (-0.0, NINF)
modf_nan = math.modf(NAN)
assert math.isnan(modf_nan[0])
assert math.isnan(modf_nan[1])

assert math.fmod(10, 1) == 0.0
assert math.fmod(10, 0.5) == 0.0
assert math.fmod(10, 1.5) == 1.0
assert math.fmod(-10, 1) == -0.0
assert math.fmod(-10, 0.5) == -0.0
assert math.fmod(-10, 1.5) == -1.0
assert math.isnan(math.fmod(NAN, 1.)) == True
assert math.isnan(math.fmod(1., NAN)) == True
assert math.isnan(math.fmod(NAN, NAN)) == True
assert_raises(ValueError, lambda: math.fmod(1., 0.))
assert_raises(ValueError, lambda: math.fmod(INF, 1.))
assert_raises(ValueError, lambda: math.fmod(NINF, 1.))
assert_raises(ValueError, lambda: math.fmod(INF, 0.))
assert math.fmod(3.0, INF) == 3.0
assert math.fmod(-3.0, INF) == -3.0
assert math.fmod(3.0, NINF) == 3.0
assert math.fmod(-3.0, NINF) == -3.0
assert math.fmod(0.0, 3.0) == 0.0
assert math.fmod(0.0, NINF) == 0.0
