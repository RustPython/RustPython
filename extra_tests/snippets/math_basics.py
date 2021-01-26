from testutils import assert_raises

assert -3 // 2 == -2
assert -3 % 2 == 1

a = 4

#print(a ** 3)
#print(a * 3)
#print(a / 2)
#print(a % 3)
#print(a - 3)
#print(-a)
#print(+a)

assert a ** 3 == 64
assert a * 3 == 12
assert a / 2 == 2
assert 2 == a / 2
assert a % 3 == 1
assert a - 3 == 1
assert -a == -4
assert +a == 4

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

assert pow(0, 0) == 1
assert pow(2, 2) == 4
assert pow(1, 2.0) == 1.0
assert pow(2.0, 1) == 2.0
assert pow(0, 10**1000) == 0
assert pow(1, 10**1000) == 1
assert pow(-1, 10**1000+1) == -1
assert pow(-1, 10**1000) == 1

assert pow(2, 4, 5) == 1
assert_raises(TypeError, pow, 2, 4, 5.0)
assert_raises(TypeError, pow, 2, 4.0, 5)
assert_raises(TypeError, pow, 2.0, 4, 5)
assert pow(2, -1, 5) == 3
assert_raises(ValueError, pow, 2, 2, 0)

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
