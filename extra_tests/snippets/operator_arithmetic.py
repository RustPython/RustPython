from testutils import assert_raises

assert -3 // 2 == -2
assert -3 % 2 == 1

a = 4

assert a ** 3 == 64
assert a * 3 == 12
assert a / 2 == 2
assert 2 == a / 2
assert a % 3 == 1
assert a - 3 == 1
assert -a == -4
assert +a == 4

# bitwise

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
