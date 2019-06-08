from testutils import assert_raises

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

assert_raises(
    ValueError,
    lambda: round(float('nan')),
    'ValueError: cannot convert float NaN to integer')
assert_raises(
    OverflowError,
    lambda: round(float('inf')),
    'OverflowError: cannot convert float NaN to integer')
assert_raises(
    OverflowError,
    lambda: round(-float('inf')),
    'OverflowError: cannot convert float NaN to integer')

assert pow(0, 0) == 1
assert pow(2, 2) == 4
assert pow(1, 2.0) == 1.0
assert pow(2.0, 1) == 2.0
assert pow(0, 10**1000) == 0
assert pow(1, 10**1000) == 1
assert pow(-1, 10**1000+1) == -1
assert pow(-1, 10**1000) == 1

assert pow(2, 4, 5) == 1
assert_raises(
    TypeError,
    lambda: pow(2, 4, 5.0),
    'pow() 3rd argument not allowed unless all arguments are integers')
assert_raises(
    TypeError,
    lambda: pow(2, 4.0, 5),
    'pow() 3rd argument not allowed unless all arguments are integers')
assert_raises(
    TypeError,
    lambda: pow(2.0, 4, 5),
    'pow() 3rd argument not allowed unless all arguments are integers')
assert_raises(
    ValueError,
    lambda: pow(2, -1, 5),
    'pow() 2nd argument cannot be negative when 3rd argument specified')
assert_raises(
    ValueError,
    lambda: pow(2, 2, 0),
    'pow() 3rd argument cannot be 0')
