assert round(0.0, 0) == 0.0
assert round(0, 0) == 0
with assertRaises(TypeError):
    round(0, 0.0)