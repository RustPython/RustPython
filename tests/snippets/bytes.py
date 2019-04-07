from testutils import assertRaises

# new
assert bytes([1,2,3])
assert bytes((1,2,3))
assert bytes(range(4))
assert b'bla'
assert bytes(3)
assert bytes("bla", "utf8")
try:
    bytes("bla")
except TypeError:
    assert True
else:
    assert False

# 

assert b'foobar'.__eq__(2) == NotImplemented
assert b'foobar'.__ne__(2) == NotImplemented
assert b'foobar'.__gt__(2) == NotImplemented
assert b'foobar'.__ge__(2) == NotImplemented
assert b'foobar'.__lt__(2) == NotImplemented
assert b'foobar'.__le__(2) == NotImplemented
