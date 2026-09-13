import struct

from testutils import assert_raises

data = struct.pack("IH", 14, 12)
assert data == bytes([14, 0, 0, 0, 12, 0])

v1, v2 = struct.unpack("IH", data)
assert v1 == 14
assert v2 == 12

data = struct.pack("<IH", 14, 12)
assert data == bytes([14, 0, 0, 0, 12, 0])

v1, v2 = struct.unpack("<IH", data)
assert v1 == 14
assert v2 == 12

data = struct.pack(">IH", 14, 12)
assert data == bytes([0, 0, 0, 14, 0, 12])

v1, v2 = struct.unpack(">IH", data)
assert v1 == 14
assert v2 == 12

data = struct.pack("3B", 65, 66, 67)
assert data == bytes([65, 66, 67])

v1, v2, v3 = struct.unpack("3B", data)
assert v1 == 65
assert v2 == 66
assert v3 == 67

with assert_raises(Exception):
    data = struct.pack("B0B", 65, 66)

with assert_raises(Exception):
    data = struct.pack("B2B", 65, 66)

data = struct.pack("B1B", 65, 66)

with assert_raises(Exception):
    struct.pack("<IH", "14", 12)

assert struct.calcsize("B") == 1
# assert struct.calcsize("<L4B") == 12

assert struct.Struct("3B").pack(65, 66, 67) == bytes([65, 66, 67])


class Indexable(object):
    def __init__(self, value):
        self._value = value

    def __index__(self):
        return self._value


data = struct.pack("B", Indexable(65))
assert data == bytes([65])

data = struct.pack("5s", b"test1")
assert data == b"test1"

data = struct.pack("3s", b"test2")
assert data == b"tes"

data = struct.pack("7s", b"test3")
assert data == b"test3\0\0"

data = struct.pack("?", True)
assert data == b"\1"

data = struct.pack("?", [])
assert data == b"\0"

assert struct.error.__module__ == "struct"
assert struct.error.__name__ == "error"

# Non-ASCII format string: error type matches CPython.
# str → UnicodeEncodeError (encoding='ascii')
# bytes → struct.error
try:
    struct.Struct("\udc00")
except UnicodeEncodeError as e:
    assert e.encoding == "ascii"
else:
    raise AssertionError("expected UnicodeEncodeError")

with assert_raises(UnicodeEncodeError):
    struct.Struct("한")

with assert_raises(struct.error):
    struct.Struct(b"\xff")


# A value the format has no room for names the format and the range it holds.
for fmt, message in (
    ("B", "'B' format requires 0 <= number <= 255"),
    ("b", "'b' format requires -128 <= number <= 127"),
    (">H", "'H' format requires 0 <= number <= 65535"),
    (">i", "'i' format requires -2147483648 <= number <= 2147483647"),
    ("N", "'N' format requires 0 <= number <= 18446744073709551615"),
    ("P", "int too large to convert"),
):
    try:
        struct.pack(fmt, 10**30)
    except struct.error as e:
        assert str(e) == message, (fmt, str(e))
    else:
        raise AssertionError(f"expected struct.error for {fmt!r}")

try:
    struct.pack("B", "x")
except struct.error as e:
    assert str(e) == "required argument is not an integer", e
else:
    raise AssertionError("expected struct.error")


# __init__ reads a new format into a Struct that already holds one.
s = struct.Struct(">h")
s.__init__(">hh")
assert s.format == ">hh"
assert s.size == 4
assert s.pack(1, 2) == b"\x00\x01\x00\x02"
assert s.unpack(b"\x00\x01\x00\x02") == (1, 2)

# A format that cannot be read leaves the Struct as it was.
for bad in ("\udc00", "$"):
    with assert_raises((UnicodeEncodeError, struct.error)):
        s.__init__(bad)
    assert s.format == ">hh"
    assert s.pack(1, 2) == b"\x00\x01\x00\x02"


# A subclass may do its own __init__ and pass the format up.
class BigShort(struct.Struct):
    def __init__(self):
        super().__init__(">h")


assert BigShort().pack(12345) == b"\x30\x39"

# Until __init__ runs there is no format to answer with.
blank = struct.Struct.__new__(struct.Struct)
assert blank.size == -1
for call in (
    lambda: blank.format,
    lambda: blank.pack(1),
    lambda: blank.unpack(b"aa"),
    lambda: blank.unpack_from(b"aaaa"),
    lambda: blank.pack_into(bytearray(4), 0, 1),
    lambda: blank.iter_unpack(b"aa"),
    lambda: repr(blank),
):
    with assert_raises(RuntimeError):
        call()


# The buffer a format asks for is sized by the format: one too large to
# allocate must raise instead of aborting.
with assert_raises(MemoryError):
    struct.pack("%dx" % (2**60))
