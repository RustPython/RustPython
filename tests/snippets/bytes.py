assert b"foobar".__eq__(2) == NotImplemented
assert b"foobar".__ne__(2) == NotImplemented
assert b"foobar".__gt__(2) == NotImplemented
assert b"foobar".__ge__(2) == NotImplemented
assert b"foobar".__lt__(2) == NotImplemented
assert b"foobar".__le__(2) == NotImplemented


# comp
a = b"abcd"
b = b"ab"
c = b"abcd"

assert a > b
assert a >= b
assert b < a
assert b <= a
assert a == c

# hash not implemented for iterator
# assert hash(iter(a)) == hash(iter(b"abcd"))

# repr
# assert repr(bytes([0, 1, 2])) == repr(b'\x00\x01\x02')
# assert (
# repr(bytes([0, 9, 10, 11, 13, 31, 32, 33, 89, 120, 255])
# == "b'\\x00\\t\\n\\x0b\\r\\x1f !Yx\\xff'")
# )
assert repr(a) == "b'abcd'"
assert len(a) == 4

assert a + b == b"abcdab"

# contains
assert b"ab" in b"abcd"
assert b"cd" in b"abcd"
assert b"abcd" in b"abcd"
assert b"a" in b"abcd"
assert b"d" in b"abcd"
assert b"dc" not in b"abcd"
assert 97 in b"abcd"
assert 150 not in b"abcd"
