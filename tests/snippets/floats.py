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
