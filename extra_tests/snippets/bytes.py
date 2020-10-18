from testutils import assert_raises

# new
assert bytes([1, 2, 3])
assert bytes((1, 2, 3))
assert bytes(range(4))
assert bytes(3)
assert b"bla"
assert bytes("bla", "utf8") == bytes("bla", encoding="utf-8") == b"bla"
with assert_raises(TypeError):
    bytes("bla")
with assert_raises(TypeError):
    bytes("bla", encoding=b"jilj")

assert (
    b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff"
    == bytes(range(0, 256))
)
assert (
    b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff"
    == bytes(range(0, 256))
)
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
with assert_raises(ValueError):
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

# upper lower, capitalize, swapcase
l = bytes(b"lower")
b = bytes(b"UPPER")
assert l.lower().islower()
assert b.upper().isupper()
assert l.capitalize() == b"Lower"
assert b.capitalize() == b"Upper"
assert bytes().capitalize() == bytes()
assert b"AaBbCc123'@/".swapcase().swapcase() == b"AaBbCc123'@/"
assert b"AaBbCc123'@/".swapcase() == b"aAbBcC123'@/"

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
with assert_raises(TypeError):
    bytes.fromhex(b"hhjjk")
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
with assert_raises(TypeError):
    b"b".center(2, "a")
with assert_raises(TypeError):
    b"b".center(2, b"ba")
with assert_raises(TypeError):
    b"b".center(b"ba")
assert b"kok".center(5, bytearray(b"x")) == b"xkokx"
b"kok".center(-5) == b"kok"


# ljust
assert [b"koki".ljust(i, b"|") for i in range(3, 10)] == [
    b"koki",
    b"koki",
    b"koki|",
    b"koki||",
    b"koki|||",
    b"koki||||",
    b"koki|||||",
]
assert [b"kok".ljust(i, b"|") for i in range(2, 10)] == [
    b"kok",
    b"kok",
    b"kok|",
    b"kok||",
    b"kok|||",
    b"kok||||",
    b"kok|||||",
    b"kok||||||",
]

b"kok".ljust(4) == b"kok "  # " test no arg"
with assert_raises(TypeError):
    b"b".ljust(2, "a")
with assert_raises(TypeError):
    b"b".ljust(2, b"ba")
with assert_raises(TypeError):
    b"b".ljust(b"ba")
assert b"kok".ljust(5, bytearray(b"x")) == b"kokxx"
assert b"kok".ljust(-5) == b"kok"

# rjust
assert [b"koki".rjust(i, b"|") for i in range(3, 10)] == [
    b"koki",
    b"koki",
    b"|koki",
    b"||koki",
    b"|||koki",
    b"||||koki",
    b"|||||koki",
]
assert [b"kok".rjust(i, b"|") for i in range(2, 10)] == [
    b"kok",
    b"kok",
    b"|kok",
    b"||kok",
    b"|||kok",
    b"||||kok",
    b"|||||kok",
    b"||||||kok",
]


b"kok".rjust(4) == b" kok"  # " test no arg"
with assert_raises(TypeError):
    b"b".rjust(2, "a")
with assert_raises(TypeError):
    b"b".rjust(2, b"ba")
with assert_raises(TypeError):
    b"b".rjust(b"ba")
assert b"kok".rjust(5, bytearray(b"x")) == b"xxkok"
assert b"kok".rjust(-5) == b"kok"


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
with assert_raises(ValueError):
    b"ilj".count(3550)
assert b"azeazerazeazopia".count(97) == 5

# join
assert (
    b"".join((b"jiljl", bytearray(b"kmoomk"), memoryview(b"aaaa")))
    == b"jiljlkmoomkaaaa"
)
with assert_raises(TypeError):
    b"".join((b"km", "kl"))

assert b"abc".join((b"123", b"xyz")) == b"123abcxyz"


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


# index find
assert b"abcd".index(b"cd") == 2
assert b"abcd".index(b"cd", 0) == 2
assert b"abcd".index(b"cd", 1) == 2
assert b"abcd".index(99) == 2
with assert_raises(ValueError):
    b"abcde".index(b"c", 3, 1)
with assert_raises(ValueError):
    b"abcd".index(b"cdaaaaa")
with assert_raises(ValueError):
    b"abcd".index(b"b", 3, 4)
with assert_raises(ValueError):
    b"abcd".index(1)


assert b"abcd".find(b"cd") == 2
assert b"abcd".find(b"cd", 0) == 2
assert b"abcd".find(b"cd", 1) == 2
assert b"abcde".find(b"c", 3, 1) == -1
assert b"abcd".find(b"cdaaaaa") == -1
assert b"abcd".find(b"b", 3, 4) == -1
assert b"abcd".find(1) == -1
assert b"abcd".find(99) == 2

assert b"abcdabcda".find(b"a") == 0
assert b"abcdabcda".rfind(b"a") == 8
assert b"abcdabcda".rfind(b"a", 2, 6) == 4
assert b"abcdabcda".rfind(b"a", None, 6) == 4
assert b"abcdabcda".rfind(b"a", 2, None) == 8
assert b"abcdabcda".index(b"a") == 0
assert b"abcdabcda".rindex(b"a") == 8


# make trans
# fmt: off
assert (
    bytes.maketrans(memoryview(b"abc"), bytearray(b"zzz"))
    == bytes([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 122, 122, 122, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255])
)
# fmt: on

# translate
assert b"hjhtuyjyujuyj".translate(bytes.maketrans(b"hj", b"ab"), b"h") == b"btuybyubuyb"
assert (
    b"hjhtuyjyujuyj".translate(bytes.maketrans(b"hj", b"ab"), b"a") == b"abatuybyubuyb"
)
assert b"hjhtuyjyujuyj".translate(bytes.maketrans(b"hj", b"ab")) == b"abatuybyubuyb"
assert b"hjhtuyfjtyhuhjuyj".translate(None, b"ht") == b"juyfjyujuyj"
assert b"hjhtuyfjtyhuhjuyj".translate(None, delete=b"ht") == b"juyfjyujuyj"


# strip lstrip rstrip
assert b" \n  spacious \n  ".strip() == b"spacious"
assert b"www.example.com".strip(b"cmowz.") == b"example"
assert b" \n  spacious   ".lstrip() == b"spacious   "
assert b"www.example.com".lstrip(b"cmowz.") == b"example.com"
assert b"   spacious \n  ".rstrip() == b"   spacious"
assert b"mississippi".rstrip(b"ipz") == b"mississ"


# split
assert b"1,2,3".split(b",") == [b"1", b"2", b"3"]
assert b"1,2,3".split(b",", maxsplit=1) == [b"1", b"2,3"]
assert b"1,2,,3,".split(b",") == [b"1", b"2", b"", b"3", b""]
assert b"1 2 3".split() == [b"1", b"2", b"3"]
assert b"1 2 3".split(maxsplit=1) == [b"1", b"2 3"]
assert b"   1   2   3   ".split() == [b"1", b"2", b"3"]
assert b"k\ruh\nfz e f".split() == [b"k", b"uh", b"fz", b"e", b"f"]
assert b"Two lines\n".split(b"\n") == [b"Two lines", b""]
assert b"".split() == []
assert b"".split(b"\n") == [b""]
assert b"\n".split(b"\n") == [b"", b""]

SPLIT_FIXTURES = [
    [
        [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[1, 2, 3], [1, 2, 3], [1, 2, 3]],
        [[1, 2, 3], [1, 2, 3], [1, 2, 3]],
        -1,
    ],
    [
        [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4, 5],
        [4, 5],
        [[1, 2, 3], [1, 2, 3], [1, 2, 3], []],
        [[1, 2, 3], [1, 2, 3], [1, 2, 3], []],
        -1,
    ],
    [
        [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 3],
        [4, 5],
        [[1, 2, 3], [1, 2, 3], [1, 2, 3], [3]],
        [[1, 2, 3], [1, 2, 3], [1, 2, 3], [3]],
        -1,
    ],
    [
        [4, 5, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[], [2, 3], [1, 2, 3], [1, 2, 3]],
        [[], [2, 3], [1, 2, 3], [1, 2, 3]],
        -1,
    ],
    [
        [1, 4, 5, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[1], [2, 3], [1, 2, 3], [1, 2, 3]],
        [[1], [2, 3], [1, 2, 3], [1, 2, 3]],
        -1,
    ],
    [
        [1, 2, 3, 4, 5, 4, 5, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[1, 2, 3], [], [], [1, 2, 3], [1, 2, 3]],
        [[1, 2, 3], [], [], [1, 2, 3], [1, 2, 3]],
        -1,
    ],
    # maxsplit
    [
        [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[1, 2, 3], [1, 2, 3, 4, 5, 1, 2, 3]],
        [[1, 2, 3, 4, 5, 1, 2, 3], [1, 2, 3]],
        1,
    ],
    [
        [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4, 5],
        [4, 5],
        [[1, 2, 3], [1, 2, 3, 4, 5, 1, 2, 3, 4, 5]],
        [[1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3], []],
        1,
    ],
    [
        [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 3],
        [4, 5],
        [[1, 2, 3], [1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 3]],
        [[1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3], [3]],
        1,
    ],
    [
        [4, 5, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[], [2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3]],
        [[4, 5, 2, 3, 4, 5, 1, 2, 3], [1, 2, 3]],
        1,
    ],
    [
        [1, 4, 5, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[1], [2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3]],
        [[1, 4, 5, 2, 3, 4, 5, 1, 2, 3], [1, 2, 3]],
        1,
    ],
    [
        [1, 2, 3, 4, 5, 4, 5, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3],
        [4, 5],
        [[1, 2, 3], [], [4, 5, 1, 2, 3, 4, 5, 1, 2, 3]],
        [[1, 2, 3, 4, 5, 4, 5], [1, 2, 3], [1, 2, 3]],
        2,
    ],
    [
        [13, 13, 13, 117, 104, 10, 102, 122, 32, 101, 102, 9, 9],
        None,
        [[117, 104], [102, 122], [101, 102]],
        [[117, 104], [102, 122], [101, 102]],
        -1,
    ],
    [
        [13, 13, 13, 117, 104, 10, 102, 122, 32, 101, 102, 9, 9],
        None,
        [[117, 104, 10, 102, 122, 32, 101, 102, 9, 9]],
        [[13, 13, 13, 117, 104, 10, 102, 122, 32, 101, 102]],
        0,
    ],
    [
        [13, 13, 13, 117, 104, 10, 102, 122, 32, 101, 102, 9, 9],
        None,
        [[117, 104], [102, 122, 32, 101, 102, 9, 9]],
        [[13, 13, 13, 117, 104, 10, 102, 122], [101, 102]],
        1,
    ],
    [
        [13, 13, 13, 117, 104, 10, 102, 122, 32, 101, 102, 9, 9],
        None,
        [[117, 104], [102, 122], [101, 102, 9, 9]],
        [[13, 13, 13, 117, 104], [102, 122], [101, 102]],
        2,
    ],
    [
        [13, 13, 13, 117, 104, 10, 10, 10, 102, 122, 32, 32, 101, 102, 9, 9],
        None,
        [[117, 104], [102, 122], [101, 102]],
        [[117, 104], [102, 122], [101, 102]],
        -1,
    ],
    [[49, 44, 50, 44, 51], [44], [[49], [50], [51]], [[49], [50], [51]], -1],
    [[49, 44, 50, 44, 51], [44], [[49], [50, 44, 51]], [[49, 44, 50], [51]], 1],
    [
        [49, 44, 50, 44, 44, 51, 44],
        [44],
        [[49], [50], [], [51], []],
        [[49], [50], [], [51], []],
        -1,
    ],
    [[49, 32, 50, 32, 51], None, [[49], [50], [51]], [[49], [50], [51]], -1],
    [[49, 32, 50, 32, 51], None, [[49], [50, 32, 51]], [[49, 32, 50], [51]], 1],
    [
        [32, 32, 32, 49, 32, 32, 32, 50, 32, 32, 32, 51, 32, 32, 32],
        None,
        [[49], [50], [51]],
        [[49], [50], [51]],
        -1,
    ],
]


# for i in SPLIT_FIXTURES:  # for not yet implemented : TypeError: Unsupported method: __next__
n_sp = 0
while n_sp < len(SPLIT_FIXTURES):
    i = SPLIT_FIXTURES[n_sp]
    sep = None if i[1] == None else bytes(i[1])
    try:
        assert bytes(i[0]).split(sep=sep, maxsplit=i[4]) == [bytes(j) for j in i[2]]
    except AssertionError:
        print(i[0], i[1], i[2])
        print(
            "Expected : ", [list(x) for x in bytes(i[0]).split(sep=sep, maxsplit=i[4])]
        )
        break

    try:
        assert bytes(i[0]).rsplit(sep=sep, maxsplit=i[4]) == [bytes(j) for j in i[3]]
    except AssertionError:
        print(i[0], i[1], i[2])
        print(
            "Expected Rev : ",
            [list(x) for x in bytes(i[0]).rsplit(sep=sep, maxsplit=i[4])],
        )
        break

    n_sp += 1


# expandtabs
a = b"\x01\x03\r\x05\t8CYZ\t\x06CYZ\t\x17cba`\n\x12\x13\x14"
assert (
    a.expandtabs() == b"\x01\x03\r\x05       8CYZ    \x06CYZ    \x17cba`\n\x12\x13\x14"
)
assert a.expandtabs(5) == b"\x01\x03\r\x05    8CYZ \x06CYZ \x17cba`\n\x12\x13\x14"
assert b"01\t012\t0123\t01234".expandtabs() == b"01      012     0123    01234"
assert b"01\t012\t0123\t01234".expandtabs(4) == b"01  012 0123    01234"
assert b"123\t123".expandtabs(-5) == b"123123"
assert b"123\t123".expandtabs(0) == b"123123"


# partition
assert b"123456789".partition(b"45") == (b"123", b"45", b"6789")
assert b"14523456789".partition(b"45") == (b"1", b"45", b"23456789")
a = b"14523456789".partition(bytearray(b"45"))
assert isinstance(a[1], bytearray)
a = b"14523456789".partition(memoryview(b"45"))
assert isinstance(a[1], memoryview)

# partition
assert b"123456789".rpartition(b"45") == (b"123", b"45", b"6789")
assert b"14523456789".rpartition(b"45") == (b"14523", b"45", b"6789")
a = b"14523456789".rpartition(bytearray(b"45"))
assert isinstance(a[1], bytearray)
a = b"14523456789".rpartition(memoryview(b"45"))
assert isinstance(a[1], memoryview)

# splitlines
assert b"ab c\n\nde fg\rkl\r\n".splitlines() == [b"ab c", b"", b"de fg", b"kl"]
assert b"ab c\n\nde fg\rkl\r\n".splitlines(keepends=True) == [
    b"ab c\n",
    b"\n",
    b"de fg\r",
    b"kl\r\n",
]
assert b"".splitlines() == []
assert b"One line\n".splitlines() == [b"One line"]

# zfill

assert b"42".zfill(5) == b"00042"
assert b"-42".zfill(5) == b"-0042"
assert b"42".zfill(1) == b"42"
assert b"42".zfill(-1) == b"42"

# replace
assert b"123456789123".replace(b"23", b"XX") == b"1XX4567891XX"
assert b"123456789123".replace(b"23", b"XX", 1) == b"1XX456789123"
assert b"123456789123".replace(b"23", b"XX", 0) == b"123456789123"
assert b"123456789123".replace(b"23", b"XX", -1) == b"1XX4567891XX"
assert b"123456789123".replace(b"23", b"") == b"14567891"
assert b"123456789123".replace(b"23", b"X") == b"1X4567891X"
assert b"rust  python".replace(b" ", b"-") == b"rust--python"
assert b"rust  python".replace(b"  ", b"-") == b"rust-python"

# title
assert b"Hello world".title() == b"Hello World"
assert (
    b"they're bill's friends from the UK".title()
    == b"They'Re Bill'S Friends From The Uk"
)


# repeat by multiply
a = b'abcd'
assert a * 0 == b''
assert a * -1 == b''
assert a * 1 == b'abcd'
assert a * 3 == b'abcdabcdabcd'
assert 3 * a == b'abcdabcdabcd'

# decode
assert b'\x72\x75\x73\x74'.decode('ascii') == 'rust'
assert b'\xc2\xae\x75\x73\x74'.decode('ascii', 'replace') == '��ust'
assert b'\xc2\xae\x75\x73\x74'.decode('ascii', 'ignore') == 'ust'
assert b'\xc2\xae\x75\x73\x74'.decode('utf-8') == '®ust'
assert b'\xc2\xae\x75\x73\x74'.decode() == '®ust'
assert b'\xe4\xb8\xad\xe6\x96\x87\xe5\xad\x97'.decode('utf-8') == '中文字'

# mod
assert b'rust%bpython%b' % (b' ', b'!') == b'rust python!'
assert b'x=%i y=%f' % (1, 2.5) == b'x=1 y=2.500000'

class A:
    def __bytes__(self):
        return b"bytess"

assert bytes(A()) == b"bytess"

# Issue #2125
b = b'abc'
assert bytes(b) is b