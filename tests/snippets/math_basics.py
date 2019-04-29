from testutils import assertRaises

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
# assert a % 3 == 1
assert a - 3 == 1
assert -a == -4
assert +a == 4

assert round(1.2) == 1
assert round(1.8) == 2
assert round(0.5) == 0
assert round(1.5) == 2
assert round(-0.5) == 0
assert round(-1.5) == -2
