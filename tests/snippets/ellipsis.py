

a = ...
b = ...
c = type(a)()  # Test singleton behavior
d = Ellipsis
e = Ellipsis

assert a is b
assert b is c
assert b is d
assert d is e

assert Ellipsis is ...
Ellipsis = 2
assert Ellipsis is not ...
