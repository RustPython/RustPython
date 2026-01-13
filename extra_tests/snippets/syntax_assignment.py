x = 1
assert x == 1

x = 1, 2, 3
assert x == (1, 2, 3)

x, y = 1, 2
assert x == 1
assert y == 2

x, y = (y, x)

assert x == 2
assert y == 1

((x, y), z) = ((1, 2), 3)

assert (x, y, z) == (1, 2, 3)

q = (1, 2, 3)
(x, y, z) = q
assert y == q[1]

x = (a, b, c) = y = q

assert (a, b, c) == q
assert x == q
assert y == q

a, *b = q
print(a)
print(b)
assert a == 1
assert b == [2, 3]

a, *b, c, d = q
print(a)
print(b)
assert a == 1
assert b == []
assert c == 2
assert d == 3

a, = [1]
assert a == 1

def g():
    yield 1337
    yield 42

a, b = g()
assert a == 1337
assert b == 42

# Variable annotations:
a: bool
b: bool = False

assert a == 1337
assert b == False

# PEP 649: In Python 3.14, __annotations__ is not automatically defined at module level
# Accessing it raises NameError
from testutils import assert_raises

with assert_raises(NameError):
    __annotations__

# Use __annotate__ to get annotations (PEP 649)
assert callable(__annotate__)
annotations = __annotate__(1)  # 1 = FORMAT_VALUE
assert annotations['a'] == bool
assert annotations['b'] == bool

n = 0

def a():
    global n
    n += 1
    return [0]

a()[0] += 1

assert n == 1
