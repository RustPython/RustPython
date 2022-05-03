from testutils import assert_raises

assert round(1.2) == 1
assert round(1.8) == 2
assert round(0.5) == 0
assert round(1.5) == 2
assert round(-0.5) == 0
assert round(-1.5) == -2

# ValueError: cannot convert float NaN to integer
assert_raises(ValueError, round, float('nan'))
# OverflowError: cannot convert float infinity to integer
assert_raises(OverflowError, round, float('inf'))
# OverflowError: cannot convert float infinity to integer
assert_raises(OverflowError, round, -float('inf'))

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
