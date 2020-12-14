

a = ...
b = ...
c = type(a)()  # Test singleton behavior
d = Ellipsis
e = Ellipsis

assert a is b
assert b is c
assert b is d
assert d is e

assert Ellipsis.__repr__() == 'Ellipsis'
assert Ellipsis.__reduce__() == 'Ellipsis'
assert type(Ellipsis).__new__(type(Ellipsis)) == Ellipsis
assert type(Ellipsis).__reduce__(Ellipsis) == 'Ellipsis'
try:
    type(Ellipsis).__new__(type(1))
except TypeError:
    pass
else:
    assert False, '`Ellipsis.__new__` should only accept `type(Ellipsis)` as argument'
try:
    type(Ellipsis).__reduce__(1)
except TypeError:
    pass
else:
    assert False, '`Ellipsis.__reduce__` should only accept `Ellipsis` as argument'

assert Ellipsis is ...
Ellipsis = 2
assert Ellipsis is not ...
