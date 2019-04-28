import math
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

# assert(math.exp(2) == math.exp(2.0))
# assert(math.exp(True) == math.exp(1.0))
#
# class Conversible():
#     def __float__(self):
#         print("Converting to float now!")
#         return 1.1111
#
# assert math.log(1.1111) == math.log(Conversible())

# roundings
assert math.trunc(1) == 1

class A(object):
    def __trunc__(self):
        return 2

assert math.trunc(A()) == 2

class A(object):
    def __trunc__(self):
        return 2.0

assert math.trunc(A()) == 2.0

class A(object):
    def __trunc__(self):
        return 'str'

assert math.trunc(A()) == 'str'
