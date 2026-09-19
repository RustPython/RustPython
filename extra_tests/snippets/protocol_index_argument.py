"""An object with `__index__` is accepted where a method takes an index."""

import sys

from testutils import assert_raises


class Index:
    def __init__(self, value=1):
        self.value = value

    def __index__(self):
        return self.value


l = [1, 2]
l.insert(Index(), 9)
assert l == [1, 9, 2]

assert "ab".rjust(Index(3)) == " ab"
assert "ab".ljust(Index(3)) == "ab "
assert "ab".center(Index(4)) == " ab "
assert "ab".zfill(Index(3)) == "0ab"
assert "a\tb".expandtabs(Index(2)) == "a b"
assert "a,b,c".split(",", Index()) == ["a", "b,c"]
assert b"a\tb".expandtabs(Index(2)) == b"a b"
assert bytearray(b"a\tb").expandtabs(Index(2)) == bytearray(b"a b")
assert (1).to_bytes(Index(2), "big") == b"\x00\x01"


# A subclass of int still works.
class MyInt(int):
    pass


assert "ab".rjust(MyInt(3)) == " ab"


# Anything without `__index__` is still rejected, with CPython's message.
def message(f):
    try:
        f()
    except TypeError as e:
        return str(e)
    raise AssertionError("expected a TypeError")


assert message(lambda: [].insert("a", 1)) == (
    "'str' object cannot be interpreted as an integer"
)
assert message(lambda: "ab".rjust(1.0)) == (
    "'float' object cannot be interpreted as an integer"
)


# An `__index__` that does not return an int is an error, not a silent cast.
class NotAnInt:
    def __index__(self):
        return "1"


with assert_raises(TypeError):
    "ab".rjust(NotAnInt())


class Raises:
    def __index__(self):
        raise ValueError("boom")


with assert_raises(ValueError):
    "ab".rjust(Raises())


# The conversion is per-argument, so a pointer or handle argument is untouched:
# ctypes dereferences the value, and CPython does not consult `__index__` there
# either. The message differs from CPython's; the refusal is the point.
try:
    import ctypes
except ImportError:
    ctypes = None

if ctypes is not None:

    class Address:
        def __index__(self):
            raise AssertionError("__index__ must not be called for an address")

    with assert_raises(TypeError):
        ctypes.c_int.from_address(Address())


# The conversion happens while the argument is bound, so an out-of-range
# __index__ is reported before a later argument is looked at. CPython raises
# OverflowError for each of these, not the error the later argument would give.
class Big:
    def __index__(self):
        return 2**200


class NotBool:
    def __bool__(self):
        raise AssertionError("signed was converted before length")


assert_raises(OverflowError, lambda: "a b".split("", Big()))
assert_raises(OverflowError, lambda: (5).to_bytes(Big(), "bogus"))
assert_raises(OverflowError, lambda: (5).to_bytes(Big(), "big", signed=NotBool()))
assert_raises(OverflowError, lambda: "a".center(Big(), 5))

# A length past isize::MAX is an OverflowError, as CPython's Py_ssize_t
# converter gives, rather than a failed allocation further in.
assert_raises(OverflowError, lambda: (5).to_bytes(Index(2**63), "big"))
assert_raises(ValueError, lambda: (5).to_bytes(Index(-1), "big"))

# An omitted optional index argument takes its default, but an explicit None
# is not an index and stays a TypeError.
assert "a b c".split(" ") == ["a", "b", "c"]
assert "a\tb".expandtabs() == "a       b"
assert (5).to_bytes() == b"\x05"
assert_raises(TypeError, lambda: "a b c".split(" ", None))
assert_raises(TypeError, lambda: "a\tb".expandtabs(None))
assert_raises(TypeError, lambda: b"a\tb".expandtabs(None))
assert_raises(TypeError, lambda: (5).to_bytes(None, "big"))

# `sys.maxsize` is a legitimate maxsplit; widening before the +1 keeps a debug
# build from overflowing on it.
assert "a b c".split(" ", sys.maxsize) == ["a", "b", "c"]
assert "a b c".rsplit(" ", sys.maxsize) == ["a", "b", "c"]
assert "a b c".split(" ", Index(sys.maxsize)) == ["a", "b", "c"]
assert b"a b".split(b" ", sys.maxsize) == [b"a", b"b"]
