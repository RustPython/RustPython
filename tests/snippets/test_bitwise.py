from testutils import assert_raises

#
# Tests
#
assert 8 >> 3 == 1
assert 8 << 3 == 64

# Left shift raises type error
assert_raises(TypeError, lambda: 1 << 0.1)
assert_raises(TypeError, lambda: 1 << "abc")

# Right shift raises type error
assert_raises(TypeError, lambda: 1 >> 0.1)
assert_raises(TypeError, lambda: 1 >> "abc")

# Left shift raises value error on negative
assert_raises(ValueError, lambda: 1 << -1)

# Right shift raises value error on negative
assert_raises(ValueError, lambda: 1 >> -1)
