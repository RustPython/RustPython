import math
from testutils import assert_raises

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
with assert_raises(AttributeError):
    assert float.__floor__
with assert_raises(AttributeError):
    assert float.__ceil__

assert math.trunc(2) == 2
assert math.ceil(3) == 3
assert math.floor(4) == 4

assert math.trunc(2.2) == 2
assert math.ceil(3.3) == 4
assert math.floor(4.4) == 4

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

assert math.frexp(float('inf')) == (float('inf'), 0)
assert str(math.frexp(float('nan'))) == str((float('nan'), 0))
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

assert math.modf(1.25) == (0.25, 1.0)
assert math.modf(-1.25) == (-0.25, -1.0)
assert math.modf(2.56) == (0.56, 2.0)
assert math.modf(-2.56) == (-0.56, -2.0)
assert math.modf(1) == (0.0, 1.0)
