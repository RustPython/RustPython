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

a = b"abcd"
b = b"ab"
c = b"abcd"

#
# repr
assert repr(bytes([0, 1, 2])) == repr(b'\x00\x01\x02')
assert (
repr(bytes([0, 1, 9, 10, 11, 13, 31, 32, 33, 89, 120, 255])
== "b'\\x00\\x01\\t\\n\\x0b\\r\\x1f !Yx\\xff'")
)
assert repr(b"abcd") == "b'abcd'"

#len
assert len(bytes("abcdé", "utf8")) == 6

#comp
assert a == b"abcd"
assert a > b
assert a >= b
assert b < a
assert b <= a

assert b'foobar'.__eq__(2) == NotImplemented
assert b'foobar'.__ne__(2) == NotImplemented
assert b'foobar'.__gt__(2) == NotImplemented
assert b'foobar'.__ge__(2) == NotImplemented
assert b'foobar'.__lt__(2) == NotImplemented
assert b'foobar'.__le__(2) == NotImplemented

#hash
hash(a) == hash(b"abcd")

#iter
[i for i in b"abcd"] == ["a", "b", "c", "d"]
