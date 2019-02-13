
a = 2
b = 2 + 4 if a < 5 else 'boe'
assert b == 6
c = 2 + 4 if a > 5 else 'boe'
assert c == 'boe'

d = lambda x, y: x > y
assert d(5, 4)

e = lambda x: 1 if x else 0
assert e(True) == 1
assert e(False) == 0

