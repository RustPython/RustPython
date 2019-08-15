from testutils import assertRaises

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

with assertRaises(TypeError):
    round(0, 0.0)
with assertRaises(TypeError):
    round(0.0, 0.0)
