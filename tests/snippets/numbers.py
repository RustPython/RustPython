x = 5
x.__init__(6)
assert x == 5

class A(int):
    pass

x = A(7)
assert x == 7
assert type(x) is A
