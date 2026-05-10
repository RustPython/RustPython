import math

from testutils import assert_raises

assert round(1.2) == 1
assert round(1.8) == 2
assert round(0.5) == 0
assert round(1.5) == 2
assert round(-0.5) == 0
assert round(-1.5) == -2

# ValueError: cannot convert float NaN to integer
assert_raises(ValueError, round, float("nan"))
# OverflowError: cannot convert float infinity to integer
assert_raises(OverflowError, round, float("inf"))
# OverflowError: cannot convert float infinity to integer
assert_raises(OverflowError, round, -float("inf"))

assert round(0) == 0
assert isinstance(round(0), int)
assert round(0.0) == 0
assert isinstance(round(0.0), int)

assert round(0, None) == 0
assert isinstance(round(0, None), int)
assert round(0.0, None) == 0
assert isinstance(round(0, None), int)

assert round(0, 0) == 0
assert isinstance(round(0, 0), int)
assert round(0.0, 0) == 0.0  # Cannot check the type
assert isinstance(round(0.0, 0), float)

with assert_raises(TypeError):
    round(0, 0.0)
with assert_raises(TypeError):
    round(0.0, 0.0)


class X:
    def __round__(self, ndigits=None):
        return 1.1


assert round(X(), 1) == 1.1
assert round(X(), None) == 1.1
assert round(X()) == 1.1


# Banker's rounding at the decimal level: values like 2.675 store as
# 2.67499... in IEEE 754, so a multiply-then-round implementation creates
# a phantom 267.5 tie that round-half-to-even snaps up to 2.68. CPython's
# `_Py_dg_dtoa` rounds at the decimal level and returns 2.67.
assert round(2.675, 2) == 2.67
assert round(2.685, 2) == 2.69
assert round(-2.675, 2) == -2.67
assert round(0.05, 1) == 0.1
assert round(0.15, 1) == 0.1
assert round(0.35, 1) == 0.3
assert round(0.45, 1) == 0.5
assert round(0.65, 1) == 0.7
assert round(0.85, 1) == 0.8
assert round(0.95, 1) == 0.9
assert round(0.645, 2) == 0.65
assert round(0.665, 2) == 0.67
assert round(0.685, 2) == 0.69
assert round(0.695, 2) == 0.69
assert round(1.685, 2) == 1.69
assert round(3.745, 2) == 3.75

# Exact-halfway ties at integer level use banker's (round-half-to-even).
assert round(0.5, 0) == 0.0
assert round(1.5, 0) == 2.0
assert round(2.5, 0) == 2.0

# Negative ndigits uses divide-then-round; banker's still applies.
assert round(1235, -1) == 1240
assert round(1245, -1) == 1240
assert round(1234.5, -1) == 1230.0
assert round(150, -2) == 200
assert round(250, -2) == 200

# NaN and infinities round to themselves with no error when ndigits is given.
assert math.isnan(round(float("nan"), 2))
assert round(float("inf"), 2) == float("inf")
assert round(float("-inf"), 2) == float("-inf")

# Signed zero is preserved through rounding.
assert math.copysign(1.0, round(-0.0, 2)) == -1.0
assert math.copysign(1.0, round(0.0, 2)) == 1.0

# Out-of-range ndigits short-circuits without overflow.
assert round(1.0, 1000) == 1.0
assert round(1.0, -1000) == 0.0
assert round(1.7976931348623157e308, 0) == 1.7976931348623157e308
