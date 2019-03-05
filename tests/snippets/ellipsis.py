

a = ...
b = ...
c = type(a)()  # Test singleton behavior

assert a is b
assert b is c
