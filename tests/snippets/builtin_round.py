from testutils import assert_raises

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
