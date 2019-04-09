from testutils import assertRaises

# new
assert bytes([1, 2, 3])
assert bytes((1, 2, 3))
assert bytes(range(4))
assert b"bla"
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
assert repr(bytes([0, 1, 2])) == repr(b"\x00\x01\x02")
assert repr(
    bytes([0, 1, 9, 10, 11, 13, 31, 32, 33, 89, 120, 255])
    == "b'\\x00\\x01\\t\\n\\x0b\\r\\x1f !Yx\\xff'"
)
assert repr(b"abcd") == "b'abcd'"

# len
assert len(bytes("abcdé", "utf8")) == 6

# comp
assert a == b"abcd"
assert a > b
assert a >= b
assert b < a
assert b <= a

assert b"foobar".__eq__(2) == NotImplemented
assert b"foobar".__ne__(2) == NotImplemented
assert b"foobar".__gt__(2) == NotImplemented
assert b"foobar".__ge__(2) == NotImplemented
assert b"foobar".__lt__(2) == NotImplemented
assert b"foobar".__le__(2) == NotImplemented

# hash
hash(a) == hash(b"abcd")

# iter
[i for i in b"abcd"] == ["a", "b", "c", "d"]

# add
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
try:
    350 in b"abcd"
except ValueError:
    pass


# getitem
d = b"abcdefghij"

assert d[1] == 98
assert d[-1] == 106
assert d[2:6] == b"cdef"
assert d[-6:] == b"efghij"
assert d[1:8:2] == b"bdfh"
assert d[8:1:-2] == b"igec"


# is_xx methods

assert bytes(b"1a23").isalnum()
assert not bytes(b"1%a23").isalnum()

assert bytes(b"abc").isalpha()
assert not bytes(b"abc1").isalpha()

# travis doesn't like this
# assert bytes(b'xyz').isascii()
# assert not bytes([128, 157, 32]).isascii()

assert bytes(b"1234567890").isdigit()
assert not bytes(b"12ab").isdigit()

l = bytes(b"lower")
b = bytes(b"UPPER")

assert l.islower()
assert not l.isupper()
assert b.isupper()
assert not bytes(b"Super Friends").islower()

assert bytes(b" \n\t").isspace()
assert not bytes(b"\td\n").isspace()

assert b.isupper()
assert not b.islower()
assert l.islower()
assert not bytes(b"tuPpEr").isupper()

assert bytes(b"Is Title Case").istitle()
assert not bytes(b"is Not title casE").istitle()
