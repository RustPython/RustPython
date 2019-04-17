from testutils import assertRaises

# new
assert bytes([1, 2, 3])
assert bytes((1, 2, 3))
assert bytes(range(4))
assert bytes(3)
assert b"bla"
assert bytes("bla", "utf8")
with assertRaises(TypeError):
    bytes("bla")

assert b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff" == bytes(range(0,256))
assert b'\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !"#$%&\'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff' == bytes(range(0,256))
assert b"omkmok\Xaa" == bytes([111, 109, 107, 109, 111, 107, 92, 88, 97, 97])


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
assert list(bytes(3)) == [0, 0, 0]

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
with assertRaises(ValueError):
    350 in b"abcd"


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

# upper lower, capitalize
l = bytes(b"lower")
b = bytes(b"UPPER")
assert l.lower().islower()
assert b.upper().isupper()
assert l.capitalize() == b"Lower"
assert b.capitalize() == b"Upper"
assert bytes().capitalize() == bytes()

# hex from hex
assert bytes([0, 1, 9, 23, 90, 234]).hex() == "000109175aea"

bytes.fromhex("62 6c7a 34350a ") == b"blz45\n"
try:
    bytes.fromhex("62 a 21")
except ValueError as e:
    str(e) == "non-hexadecimal number found in fromhex() arg at position 4"
try:
    bytes.fromhex("6Z2")
except ValueError as e:
    str(e) == "non-hexadecimal number found in fromhex() arg at position 1"

# center
assert [b"koki".center(i, b"|") for i in range(3, 10)] == [
    b"koki",
    b"koki",
    b"|koki",
    b"|koki|",
    b"||koki|",
    b"||koki||",
    b"|||koki||",
]

assert [b"kok".center(i, b"|") for i in range(2, 10)] == [
    b"kok",
    b"kok",
    b"kok|",
    b"|kok|",
    b"|kok||",
    b"||kok||",
    b"||kok|||",
    b"|||kok|||",
]
b"kok".center(4) == b" kok"  # " test no arg"
with assertRaises(TypeError):
    b"b".center(2, "a")
with assertRaises(TypeError):
    b"b".center(2, b"ba")
b"kok".center(5, bytearray(b"x"))
b"kok".center(-5)

# count
assert b"azeazerazeazopia".count(b"aze") == 3
assert b"azeazerazeazopia".count(b"az") == 4
assert b"azeazerazeazopia".count(b"a") == 5
assert b"123456789".count(b"") == 10
assert b"azeazerazeazopia".count(bytearray(b"aze")) == 3
assert b"azeazerazeazopia".count(memoryview(b"aze")) == 3
assert b"azeazerazeazopia".count(memoryview(b"aze"), 1, 9) == 1
assert b"azeazerazeazopia".count(b"aze", None, None) == 3
assert b"azeazerazeazopia".count(b"aze", 2, None) == 2
assert b"azeazerazeazopia".count(b"aze", 2) == 2
assert b"azeazerazeazopia".count(b"aze", None, 7) == 2
assert b"azeazerazeazopia".count(b"aze", None, 7) == 2
assert b"azeazerazeazopia".count(b"aze", 2, 7) == 1
assert b"azeazerazeazopia".count(b"aze", -13, -10) == 1
assert b"azeazerazeazopia".count(b"aze", 1, 10000) == 2
with assertRaises(ValueError):
    b"ilj".count(3550)
assert b"azeazerazeazopia".count(97) == 5

# join
assert (
    b"".join((b"jiljl", bytearray(b"kmoomk"), memoryview(b"aaaa")))
    == b"jiljlkmoomkaaaa"
)
with assertRaises(TypeError):
    b"".join((b"km", "kl"))


# endswith startswith
assert b"abcde".endswith(b"de")
assert b"abcde".endswith(b"")
assert not b"abcde".endswith(b"zx")
assert b"abcde".endswith(b"bc", 0, 3)
assert not b"abcde".endswith(b"bc", 2, 3)
assert b"abcde".endswith((b"c", b"de"))

assert b"abcde".startswith(b"ab")
assert b"abcde".startswith(b"")
assert not b"abcde".startswith(b"zx")
assert b"abcde".startswith(b"cd", 2)
assert not b"abcde".startswith(b"cd", 1, 4)
assert b"abcde".startswith((b"a", b"bc"))
