x = 5
x.__init__(6)
assert x == 5

class A(int):
    pass

x = A(7)
assert x == 7
assert type(x) is A

assert int(2).__bool__() == True
assert int(0.5).__bool__() == False
assert int(-1).__bool__() == True

assert int(0).__invert__() == -1
assert int(-3).__invert__() == 2
assert int(4).__invert__() == -5
