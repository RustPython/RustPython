x = 5
x.__init__(6)
assert x == 5


class A(int):
    pass


x = A(7)
assert x == 7
assert type(x) is A

assert int(2).__index__() == 2
assert int(2).__trunc__() == 2
assert int(2).__ceil__() == 2
assert int(2).__floor__() == 2
assert int(2).__round__() == 2
assert int(2).__round__(3) == 2
assert int(-2).__index__() == -2
assert int(-2).__trunc__() == -2
assert int(-2).__ceil__() == -2
assert int(-2).__floor__() == -2
assert int(-2).__round__() == -2
assert int(-2).__round__(3) == -2

assert round(10) == 10
assert round(10, 2) == 10
assert round(10, -1) == 10

assert int(2).__bool__() == True
assert int(0.5).__bool__() == False
assert int(-1).__bool__() == True

assert int(0).__invert__() == -1
assert int(-3).__invert__() == 2
assert int(4).__invert__() == -5

assert int(0).__rxor__(0) == 0
assert int(1).__rxor__(0) == 1
assert int(0).__rxor__(1) == 1
assert int(1).__rxor__(1) == 0
assert int(3).__rxor__(-3) == -2
assert int(3).__rxor__(4) == 7
