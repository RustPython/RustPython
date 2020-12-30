from testutils import assert_raises
import pickle
import sys

# new
assert bytearray([1, 2, 3])
assert bytearray((1, 2, 3))
assert bytearray(range(4))
assert bytearray(3)
assert b"bla"
assert (
    bytearray("bla", "utf8") == bytearray("bla", encoding="utf-8") == bytearray(b"bla")
)
with assert_raises(TypeError):
    bytearray("bla")
with assert_raises(TypeError):
    bytearray("bla", encoding=b"jilj")

assert bytearray(
    b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff"
) == bytearray(range(0, 256))
assert bytearray(
    b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff"
) == bytearray(range(0, 256))
assert bytearray(b"omkmok\Xaa") == bytearray(
    [111, 109, 107, 109, 111, 107, 92, 88, 97, 97]
)


a = bytearray(b"abcd")
b = bytearray(b"ab")
c = bytearray(b"abcd")


# repr
assert repr(bytearray([0, 1, 2])) == repr(bytearray(b"\x00\x01\x02"))
assert (
    repr(bytearray([0, 1, 9, 10, 11, 13, 31, 32, 33, 89, 120, 255]))
    == "bytearray(b'\\x00\\x01\\t\\n\\x0b\\r\\x1f !Yx\\xff')"
)
assert repr(bytearray(b"abcd")) == "bytearray(b'abcd')"

# len
assert len(bytearray("abcdé", "utf8")) == 6

# comp
assert a == b"abcd"
assert a > b
assert a >= b
assert b < a
assert b <= a

assert bytearray(b"foobar").__eq__(2) == NotImplemented
assert bytearray(b"foobar").__ne__(2) == NotImplemented
assert bytearray(b"foobar").__gt__(2) == NotImplemented
assert bytearray(b"foobar").__ge__(2) == NotImplemented
assert bytearray(b"foobar").__lt__(2) == NotImplemented
assert bytearray(b"foobar").__le__(2) == NotImplemented

# # hash
with assert_raises(TypeError):
    hash(bytearray(b"abcd"))  # unashable

# # iter
[i for i in bytearray(b"abcd")] == ["a", "b", "c", "d"]
assert list(bytearray(3)) == [0, 0, 0]

# add
assert a + b == bytearray(b"abcdab")

# contains
assert bytearray(b"ab") in bytearray(b"abcd")
assert bytearray(b"cd") in bytearray(b"abcd")
assert bytearray(b"abcd") in bytearray(b"abcd")
assert bytearray(b"a") in bytearray(b"abcd")
assert bytearray(b"d") in bytearray(b"abcd")
assert bytearray(b"dc") not in bytearray(b"abcd")
assert 97 in bytearray(b"abcd")
assert 150 not in bytearray(b"abcd")
with assert_raises(ValueError):
    350 in bytearray(b"abcd")


# getitem
d = bytearray(b"abcdefghij")

assert d[1] == 98
assert d[-1] == 106
assert d[2:6] == bytearray(b"cdef")
assert d[-6:] == bytearray(b"efghij")
assert d[1:8:2] == bytearray(b"bdfh")
assert d[8:1:-2] == bytearray(b"igec")


# # is_xx methods

assert bytearray(b"1a23").isalnum()
assert not bytearray(b"1%a23").isalnum()

assert bytearray(b"abc").isalpha()
assert not bytearray(b"abc1").isalpha()

# travis doesn't like this
# assert bytearray(b'xyz').isascii()
# assert not bytearray([128, 157, 32]).isascii()

assert bytearray(b"1234567890").isdigit()
assert not bytearray(b"12ab").isdigit()

l = bytearray(b"lower")
b = bytearray(b"UPPER")

assert l.islower()
assert not l.isupper()
assert b.isupper()
assert not bytearray(b"Super Friends").islower()

assert bytearray(b" \n\t").isspace()
assert not bytearray(b"\td\n").isspace()

assert b.isupper()
assert not b.islower()
assert l.islower()
assert not bytearray(b"tuPpEr").isupper()

assert bytearray(b"Is Title Case").istitle()
assert not bytearray(b"is Not title casE").istitle()

# upper lower, capitalize, swapcase
l = bytearray(b"lower")
b = bytearray(b"UPPER")
assert l.lower().islower()
assert b.upper().isupper()
assert l.capitalize() == b"Lower"
assert b.capitalize() == b"Upper"
assert bytearray().capitalize() == bytearray()
assert b"AaBbCc123'@/".swapcase().swapcase() == b"AaBbCc123'@/"
assert b"AaBbCc123'@/".swapcase() == b"aAbBcC123'@/"

# # hex from hex
assert bytearray([0, 1, 9, 23, 90, 234]).hex() == "000109175aea"

bytearray.fromhex("62 6c7a 34350a ") == b"blz45\n"
try:
    bytearray.fromhex("62 a 21")
except ValueError as e:
    str(e) == "non-hexadecimal number found in fromhex() arg at position 4"
try:
    bytearray.fromhex("6Z2")
except ValueError as e:
    str(e) == "non-hexadecimal number found in fromhex() arg at position 1"
with assert_raises(TypeError):
    bytearray.fromhex(b"hhjjk")
# center
assert [bytearray(b"koki").center(i, b"|") for i in range(3, 10)] == [
    b"koki",
    b"koki",
    b"|koki",
    b"|koki|",
    b"||koki|",
    b"||koki||",
    b"|||koki||",
]

assert [bytearray(b"kok").center(i, b"|") for i in range(2, 10)] == [
    b"kok",
    b"kok",
    b"kok|",
    b"|kok|",
    b"|kok||",
    b"||kok||",
    b"||kok|||",
    b"|||kok|||",
]
bytearray(b"kok").center(4) == b" kok"  # " test no arg"
with assert_raises(TypeError):
    bytearray(b"b").center(2, "a")
with assert_raises(TypeError):
    bytearray(b"b").center(2, b"ba")
with assert_raises(TypeError):
    bytearray(b"b").center(b"ba")
assert bytearray(b"kok").center(5, bytearray(b"x")) == b"xkokx"
bytearray(b"kok").center(-5) == b"kok"


# ljust
assert [bytearray(b"koki").ljust(i, b"|") for i in range(3, 10)] == [
    b"koki",
    b"koki",
    b"koki|",
    b"koki||",
    b"koki|||",
    b"koki||||",
    b"koki|||||",
]
assert [bytearray(b"kok").ljust(i, b"|") for i in range(2, 10)] == [
    b"kok",
    b"kok",
    b"kok|",
    b"kok||",
    b"kok|||",
    b"kok||||",
    b"kok|||||",
    b"kok||||||",
]

bytearray(b"kok").ljust(4) == b"kok "  # " test no arg"
with assert_raises(TypeError):
    bytearray(b"b").ljust(2, "a")
with assert_raises(TypeError):
    bytearray(b"b").ljust(2, b"ba")
with assert_raises(TypeError):
    bytearray(b"b").ljust(b"ba")
assert bytearray(b"kok").ljust(5, bytearray(b"x")) == b"kokxx"
assert bytearray(b"kok").ljust(-5) == b"kok"

# rjust
assert [bytearray(b"koki").rjust(i, b"|") for i in range(3, 10)] == [
    b"koki",
    b"koki",
    b"|koki",
    b"||koki",
    b"|||koki",
    b"||||koki",
    b"|||||koki",
]
assert [bytearray(b"kok").rjust(i, b"|") for i in range(2, 10)] == [
    b"kok",
    b"kok",
    b"|kok",
    b"||kok",
    b"|||kok",
    b"||||kok",
    b"|||||kok",
    b"||||||kok",
]


bytearray(b"kok").rjust(4) == b" kok"  # " test no arg"
with assert_raises(TypeError):
    bytearray(b"b").rjust(2, "a")
with assert_raises(TypeError):
    bytearray(b"b").rjust(2, b"ba")
with assert_raises(TypeError):
    bytearray(b"b").rjust(b"ba")
assert bytearray(b"kok").rjust(5, bytearray(b"x")) == b"xxkok"
assert bytearray(b"kok").rjust(-5) == b"kok"


# count
assert bytearray(b"azeazerazeazopia").count(b"aze") == 3
assert bytearray(b"azeazerazeazopia").count(b"az") == 4
assert bytearray(b"azeazerazeazopia").count(b"a") == 5
assert bytearray(b"123456789").count(b"") == 10
assert bytearray(b"azeazerazeazopia").count(bytearray(b"aze")) == 3
assert bytearray(b"azeazerazeazopia").count(memoryview(b"aze")) == 3
assert bytearray(b"azeazerazeazopia").count(memoryview(b"aze"), 1, 9) == 1
assert bytearray(b"azeazerazeazopia").count(b"aze", None, None) == 3
assert bytearray(b"azeazerazeazopia").count(b"aze", 2, None) == 2
assert bytearray(b"azeazerazeazopia").count(b"aze", 2) == 2
assert bytearray(b"azeazerazeazopia").count(b"aze", None, 7) == 2
assert bytearray(b"azeazerazeazopia").count(b"aze", None, 7) == 2
assert bytearray(b"azeazerazeazopia").count(b"aze", 2, 7) == 1
assert bytearray(b"azeazerazeazopia").count(b"aze", -13, -10) == 1
assert bytearray(b"azeazerazeazopia").count(b"aze", 1, 10000) == 2
with assert_raises(ValueError):
    bytearray(b"ilj").count(3550)
assert bytearray(b"azeazerazeazopia").count(97) == 5

# join
assert bytearray(b"").join(
    (b"jiljl", bytearray(b"kmoomk"), memoryview(b"aaaa"))
) == bytearray(b"jiljlkmoomkaaaa")
with assert_raises(TypeError):
    bytearray(b"").join((b"km", "kl"))
assert bytearray(b"abc").join((
    bytearray(b"123"), bytearray(b"xyz")
)) == bytearray(b"123abcxyz")


# endswith startswith
assert bytearray(b"abcde").endswith(b"de")
assert bytearray(b"abcde").endswith(b"")
assert not bytearray(b"abcde").endswith(b"zx")
assert bytearray(b"abcde").endswith(b"bc", 0, 3)
assert not bytearray(b"abcde").endswith(b"bc", 2, 3)
assert bytearray(b"abcde").endswith((b"c", bytearray(b"de")))

assert bytearray(b"abcde").startswith(b"ab")
assert bytearray(b"abcde").startswith(b"")
assert not bytearray(b"abcde").startswith(b"zx")
assert bytearray(b"abcde").startswith(b"cd", 2)
assert not bytearray(b"abcde").startswith(b"cd", 1, 4)
assert bytearray(b"abcde").startswith((b"a", bytearray(b"bc")))


# index find
assert bytearray(b"abcd").index(b"cd") == 2
assert bytearray(b"abcd").index(b"cd", 0) == 2
assert bytearray(b"abcd").index(b"cd", 1) == 2
assert bytearray(b"abcd").index(99) == 2
with assert_raises(ValueError):
    bytearray(b"abcde").index(b"c", 3, 1)
with assert_raises(ValueError):
    bytearray(b"abcd").index(b"cdaaaaa")
with assert_raises(ValueError):
    bytearray(b"abcd").index(b"b", 3, 4)
with assert_raises(ValueError):
    bytearray(b"abcd").index(1)


assert bytearray(b"abcd").find(b"cd") == 2
assert bytearray(b"abcd").find(b"cd", 0) == 2
assert bytearray(b"abcd").find(b"cd", 1) == 2
assert bytearray(b"abcde").find(b"c", 3, 1) == -1
assert bytearray(b"abcd").find(b"cdaaaaa") == -1
assert bytearray(b"abcd").find(b"b", 3, 4) == -1
assert bytearray(b"abcd").find(1) == -1
assert bytearray(b"abcd").find(99) == 2

assert bytearray(b"abcdabcda").find(b"a") == 0
assert bytearray(b"abcdabcda").rfind(b"a") == 8
assert bytearray(b"abcdabcda").rfind(b"a", 2, 6) == 4
assert bytearray(b"abcdabcda").rfind(b"a", None, 6) == 4
assert bytearray(b"abcdabcda").rfind(b"a", 2, None) == 8
assert bytearray(b"abcdabcda").index(b"a") == 0
assert bytearray(b"abcdabcda").rindex(b"a") == 8


# make trans
# fmt: off
assert (
    bytearray.maketrans(memoryview(b"abc"), bytearray(b"zzz"))
    == bytes([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 122, 122, 122, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255])
)
# fmt: on

# translate
assert bytearray(b"hjhtuyjyujuyj").translate(
    bytearray.maketrans(b"hj", bytearray(b"ab")), bytearray(b"h")
) == bytearray(b"btuybyubuyb")
assert bytearray(b"hjhtuyjyujuyj").translate(
    bytearray.maketrans(b"hj", bytearray(b"ab")), bytearray(b"a")
) == bytearray(b"abatuybyubuyb")
assert bytearray(b"hjhtuyjyujuyj").translate(
    bytearray.maketrans(b"hj", bytearray(b"ab"))
) == bytearray(b"abatuybyubuyb")
assert bytearray(b"hjhtuyfjtyhuhjuyj").translate(None, bytearray(b"ht")) == bytearray(
    b"juyfjyujuyj"
)
assert bytearray(b"hjhtuyfjtyhuhjuyj").translate(None, delete=b"ht") == bytearray(
    b"juyfjyujuyj"
)


# strip lstrip rstrip
assert bytearray(b" \n  spacious \n  ").strip() == bytearray(b"spacious")
assert bytearray(b"www.example.com").strip(b"cmowz.") == bytearray(b"example")
assert bytearray(b" \n  spacious   ").lstrip() == bytearray(b"spacious   ")
assert bytearray(b"www.example.com").lstrip(b"cmowz.") == bytearray(b"example.com")
assert bytearray(b"   spacious \n  ").rstrip() == bytearray(b"   spacious")
assert bytearray(b"mississippi").rstrip(b"ipz") == bytearray(b"mississ")



# split
assert bytearray(b"1,2,3").split(bytearray(b",")) == [bytearray(b"1"), bytearray(b"2"), bytearray(b"3")]
assert bytearray(b"1,2,3").split(bytearray(b","), maxsplit=1) == [bytearray(b"1"), bytearray(b"2,3")]
assert bytearray(b"1,2,,3,").split(bytearray(b",")) == [bytearray(b"1"), bytearray(b"2"), bytearray(b""), bytearray(b"3"), bytearray(b"")]
assert bytearray(b"1 2 3").split() == [bytearray(b"1"), bytearray(b"2"), bytearray(b"3")]
assert bytearray(b"1 2 3").split(maxsplit=1) == [bytearray(b"1"), bytearray(b"2 3")]
assert bytearray(b"   1   2   3   ").split() == [bytearray(b"1"), bytearray(b"2"), bytearray(b"3")]
assert bytearray(b"k\ruh\nfz e f").split() == [bytearray(b"k"), bytearray(b"uh"), bytearray(b"fz"), bytearray(b"e"), bytearray(b"f")]
assert bytearray(b"Two lines\n").split(bytearray(b"\n")) == [bytearray(b"Two lines"), bytearray(b"")]
assert bytearray(b"").split() == []
assert bytearray(b"").split(bytearray(b"\n")) == [bytearray(b"")]
assert bytearray(b"\n").split(bytearray(b"\n")) == [bytearray(b""), bytearray(b"")]

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
    sep = None if i[1] == None else bytearray(i[1])
    try:
        assert bytearray(i[0]).split(sep=sep, maxsplit=i[4]) == [bytearray(j) for j in i[2]]
    except AssertionError:
        print(i[0], i[1], i[2])
        print(
            "Expected : ", [list(x) for x in bytearray(i[0]).split(sep=sep, maxsplit=i[4])]
        )
        break

    try:
        assert bytearray(i[0]).rsplit(sep=sep, maxsplit=i[4]) == [bytearray(j) for j in i[3]]
    except AssertionError:
        print(i[0], i[1], i[2])
        print(
            "Expected Rev : ",
            [list(x) for x in bytearray(i[0]).rsplit(sep=sep, maxsplit=i[4])],
        )
        break

    n_sp += 1


# expandtabs
a = bytearray(b"\x01\x03\r\x05\t8CYZ\t\x06CYZ\t\x17cba`\n\x12\x13\x14")
assert (
    a.expandtabs() == bytearray(b"\x01\x03\r\x05       8CYZ    \x06CYZ    \x17cba`\n\x12\x13\x14")
)
assert a.expandtabs(5) == bytearray(b"\x01\x03\r\x05    8CYZ \x06CYZ \x17cba`\n\x12\x13\x14")
assert bytearray(b"01\t012\t0123\t01234").expandtabs() == bytearray(b"01      012     0123    01234")
assert bytearray(b"01\t012\t0123\t01234").expandtabs(4) == bytearray(b"01  012 0123    01234")
assert bytearray(b"123\t123").expandtabs(-5) == bytearray(b"123123")
assert bytearray(b"123\t123").expandtabs(0) == bytearray(b"123123")


# # partition
assert bytearray(b"123456789").partition(b"45") == ((b"123"), bytearray(b"45"), bytearray(b"6789"))
assert bytearray(b"14523456789").partition(b"45") == ((b"1"), bytearray(b"45"), bytearray(b"23456789"))
a = bytearray(b"14523456789").partition(b"45")
assert isinstance(a[1], bytearray)
a = bytearray(b"14523456789").partition(memoryview(b"45"))
assert isinstance(a[1], bytearray)

# partition
assert bytearray(b"123456789").rpartition(bytearray(b"45")) == ((bytearray(b"123")), bytearray(b"45"), bytearray(b"6789"))
assert bytearray(b"14523456789").rpartition(bytearray(b"45")) == ((bytearray(b"14523")), bytearray(b"45"), bytearray(b"6789"))
a = bytearray(b"14523456789").rpartition(b"45")
assert isinstance(a[1], bytearray)
a = bytearray(b"14523456789").rpartition(memoryview(b"45"))
assert isinstance(a[1], bytearray)

# splitlines
assert bytearray(b"ab c\n\nde fg\rkl\r\n").splitlines() == [bytearray(b"ab c"), bytearray(b""), bytearray(b"de fg"), bytearray(b"kl")]
assert bytearray(b"ab c\n\nde fg\rkl\r\n").splitlines(keepends=True) == [
    bytearray(b"ab c\n"),
    bytearray(b"\n"),
    bytearray(b"de fg\r"),
    bytearray(b"kl\r\n"),
]
assert bytearray(b"").splitlines() == []
assert bytearray(b"One line\n").splitlines() == [b"One line"]

# zfill

assert bytearray(b"42").zfill(5) == bytearray(b"00042")
assert bytearray(b"-42").zfill(5) == bytearray(b"-0042")
assert bytearray(b"42").zfill(1) == bytearray(b"42")
assert bytearray(b"42").zfill(-1) == bytearray(b"42")

# replace
assert bytearray(b"123456789123").replace(b"23",b"XX") ==bytearray(b'1XX4567891XX')
assert bytearray(b"123456789123").replace(b"23",b"XX", 1) ==bytearray(b'1XX456789123')
assert bytearray(b"123456789123").replace(b"23",b"XX", 0) == bytearray(b"123456789123")
assert bytearray(b"123456789123").replace(b"23",b"XX", -1) ==bytearray(b'1XX4567891XX')
assert bytearray(b"123456789123").replace(b"23", bytearray(b"")) == bytearray(b"14567891")


# clear
a = bytearray(b"abcd")
a.clear()
assert len(a) == 0

b = bytearray(b"test")
assert len(b) == 4
b.pop()
assert len(b) == 3

c = bytearray([123, 255, 111])
assert len(c) == 3
c.pop()
assert len(c) == 2
c.pop()
c.pop()

try:
    c.pop()
except IndexError:
    pass
else:
    assert False

a = bytearray(b"appen")
assert len(a) == 5
a.append(100)
assert a == bytearray(b"append")
assert len(a) == 6
assert a.pop() == 100

# title
assert bytearray(b"Hello world").title() == bytearray(b"Hello World")
assert (
    bytearray(b"they're bill's friends from the UK").title()
    == bytearray(b"They'Re Bill'S Friends From The Uk")
)


# repeat by multiply
a = bytearray(b'abcd')
assert a * 0 == bytearray(b'')
assert a * -1 == bytearray(b'')
assert a * 1 == bytearray(b'abcd')
assert a * 3 == bytearray(b'abcdabcdabcd')
assert 3 * a == bytearray(b'abcdabcdabcd')

a = bytearray(b'abcd')
a.__imul__(3)
assert a == bytearray(b'abcdabcdabcd')
a.__imul__(0)
assert a == bytearray(b'')


# copy
a = bytearray(b"my bytearray")
b = a.copy()
assert a == b
assert a is not b
b.append(100)
assert a != b


# extend
a = bytearray(b"hello,")
# any iterable of ints should work
a.extend([32, 119, 111, 114])
a.extend(b"ld")
assert a == bytearray(b"hello, world")


# insert
a = bytearray(b"hello, world")
a.insert(0, 119)
assert a == bytearray(b"whello, world"), a
# -1 is not at the end, but one before
a.insert(-1, 119)
assert a == bytearray(b"whello, worlwd"), a
# inserting before the beginning just inserts at the beginning
a.insert(-1000, 111)
assert a == bytearray(b"owhello, worlwd"), a
# inserting after the end just inserts at the end
a.insert(1000, 111)
assert a == bytearray(b"owhello, worlwdo"), a


# remove
a = bytearray(b'abcdabcd')
a.remove(99)  # the letter c
# Only the first is removed
assert a == bytearray(b'abdabcd')


# reverse
a = bytearray(b'hello, world')
a.reverse()
assert a == bytearray(b'dlrow ,olleh')

# __setitem__
a = bytearray(b'test')
a[0] = 1
assert a == bytearray(b'\x01est')
with assert_raises(TypeError):
    a[0] = b'a'
with assert_raises(TypeError):
    a[0] = memoryview(b'a')
a[:2] = [0, 9]
assert a == bytearray(b'\x00\x09st')
a[1:3] = b'test'
assert a == bytearray(b'\x00testt')
a[:6] = memoryview(b'test')
assert a == bytearray(b'test')

# mod
assert bytearray('rust%bpython%b', 'utf-8') % (b' ', b'!') == bytearray(b'rust python!')
assert bytearray('x=%i y=%f', 'utf-8') % (1, 2.5) == bytearray(b'x=1 y=2.500000')

# eq, ne
a = bytearray(b'hello, world')
b = a.copy()
assert a.__ne__(b) is False
b = bytearray(b'my bytearray')
assert a.__ne__(b) is True

# pickle
a = bytearray(b'\xffab\x80\0\0\370\0\0')
assert pickle.dumps(a, 0) == b'c__builtin__\nbytearray\np0\n(c_codecs\nencode\np1\n(V\xffab\x80\\u0000\\u0000\xf8\\u0000\\u0000\np2\nVlatin1\np3\ntp4\nRp5\ntp6\nRp7\n.'
assert pickle.dumps(a, 1) == b'c__builtin__\nbytearray\nq\x00(c_codecs\nencode\nq\x01(X\x0c\x00\x00\x00\xc3\xbfab\xc2\x80\x00\x00\xc3\xb8\x00\x00q\x02X\x06\x00\x00\x00latin1q\x03tq\x04Rq\x05tq\x06Rq\x07.'
assert pickle.dumps(a, 2) == b'\x80\x02c__builtin__\nbytearray\nq\x00c_codecs\nencode\nq\x01X\x0c\x00\x00\x00\xc3\xbfab\xc2\x80\x00\x00\xc3\xb8\x00\x00q\x02X\x06\x00\x00\x00latin1q\x03\x86q\x04Rq\x05\x85q\x06Rq\x07.'
assert pickle.dumps(a, 3) == b'\x80\x03cbuiltins\nbytearray\nq\x00C\t\xffab\x80\x00\x00\xf8\x00\x00q\x01\x85q\x02Rq\x03.'
assert pickle.dumps(a, 4) == b'\x80\x04\x95*\x00\x00\x00\x00\x00\x00\x00\x8c\x08builtins\x94\x8c\tbytearray\x94\x93\x94C\t\xffab\x80\x00\x00\xf8\x00\x00\x94\x85\x94R\x94.'

# pickle with subclass
class A(bytes):
    pass

a = A()
a.x = 10
a.y = A(b'123')
b = pickle.loads(pickle.dumps(a, 4))
assert type(a) == type(b)
assert a.x == b.x
assert a.y == b.y
assert a == b

class B(bytearray):
    pass

a = B()
a.x = 10
a.y = B(b'123')
b = pickle.loads(pickle.dumps(a, 4))
assert type(a) == type(b)
assert a.x == b.x
assert a.y == b.y
assert a == b

a = bytearray()
for i in range(-1, 2, 1):
    assert_raises(IndexError, lambda: a[-sys.maxsize - i], _msg='bytearray index out of range')