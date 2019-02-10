1 + 1.1

a = 1.2
b = 1.3
c = 1.2
assert a < b
assert not b < a
assert a <= b
assert a <= c

assert b > a
assert not a > b
assert not a > c
assert b >= a
assert c >= a
assert not a >= b

assert a + b == 2.5
assert a - c == 0
assert a / c == 1

assert a < 5
assert a <= 5
try:
    assert a < 'a'
except TypeError:
    pass
try:
    assert a <= 'a'
except TypeError:
    pass
assert a > 1
assert a >= 1
try:
    assert a > 'a'
except TypeError:
    pass
try:
    assert a >= 'a'
except TypeError:
    pass
