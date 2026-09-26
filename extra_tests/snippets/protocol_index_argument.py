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


# Primitive integer arguments accept `__index__`.
assert chr(Index(65)) == chr(65)
assert bin(Index(3)) == bin(3)
assert [1, 2, 3].pop(Index(0)) == 1
assert b"ab".center(Index(4)) == b"ab".center(4)
assert (1, 2, 3).index(2, Index(1)) == 1

import math

assert math.factorial(Index(5)) == math.factorial(5)

import array

arr = array.array("L", [1, 2, 3])
assert arr.pop(Index(0)) == 1
arr = array.array("L")
arr.append(Index(4))
assert list(arr) == [4]

import os

assert os.strerror(Index(0)) == os.strerror(0)
if hasattr(os, "major"):
    assert os.major(Index(0x3000000)) == os.major(0x3000000)
try:
    os.read(Index(-1), Index(1))
except OSError:
    pass
else:
    raise AssertionError(
        "os.read should fail on a bad descriptor, not accept it silently"
    )

import time

assert time.localtime(Index(0)).tm_year == time.localtime(0).tm_year

import socket

assert socket.htons(Index(1)) == socket.htons(1)

import zlib

assert zlib.crc32(b"a", Index(0)) == zlib.crc32(b"a", 0)

import io

bio = io.BytesIO(b"abcdef")
assert bio.seek(Index(2)) == bio.seek(2)

old_limit = sys.getrecursionlimit()
try:
    sys.setrecursionlimit(Index(old_limit))
    assert sys.getrecursionlimit() == old_limit
finally:
    sys.setrecursionlimit(old_limit)

import signal

if hasattr(signal, "alarm"):
    signal.alarm(Index(0))

import select

# Windows select() refuses three empty lists.
if sys.platform != "win32":
    assert select.select([], [], [], Index(0)) == ([], [], [])


class BoomIndex:
    def __index__(self):
        raise KeyError("boom")


if hasattr(select, "poll"):
    poller = select.poll()
    poller.register(0, Index(select.POLLIN))
    poller.unregister(0)

    assert message(lambda: poller.register(0, 1.0)) == (
        "'float' object cannot be interpreted as an integer"
    )
    assert message(lambda: poller.register(0, "x")) == (
        "'str' object cannot be interpreted as an integer"
    )
    with assert_raises(KeyError):
        poller.register(0, BoomIndex())


# An int subclass is used directly; its `__index__` is not called.
class OverrideIndex(int):
    def __index__(self):
        raise AssertionError("__index__ must not be called on an int subclass")


assert chr(OverrideIndex(66)) == "B"
assert bin(OverrideIndex(4)) == "0b100"
assert [9].pop(OverrideIndex(0)) == 9
assert math.factorial(OverrideIndex(3)) == 6


# A float is still not an integer.
assert message(lambda: chr(1.0)) == "'float' object cannot be interpreted as an integer"
assert (
    message(lambda: [1].pop(1.0))
    == "'float' object cannot be interpreted as an integer"
)


# Sites that check for an int themselves keep refusing `__index__`.
assert message(lambda: math.ldexp(1.0, Index(2))) == (
    "Expected an int as second argument to ldexp."
)

import collections

with assert_raises(TypeError):
    collections.deque(maxlen=Index(2))


class HashIndex:
    def __hash__(self):
        return Index(1)


assert message(lambda: hash(HashIndex())) == "__hash__ method should return an integer"

import _thread

assert (
    message(lambda: _thread._make_thread_handle(Index(1))) == "ident must be an integer"
)

if ctypes is not None:
    import _ctypes

    with assert_raises(TypeError):
        ctypes.Structure.from_address(Index(0))
    with assert_raises(TypeError):
        ctypes.Union.from_address(Index(0))
    for name in (
        "PyObj_FromPtr",
        "dlsym",
        "dlclose",
        "call_function",
        "call_cdeclfunction",
        "FreeLibrary",
    ):
        fn = getattr(_ctypes, name, None)
        # Windows handle arguments are left out of this change.
        if fn is None or (name == "FreeLibrary" and sys.platform == "win32"):
            continue
        with assert_raises(TypeError):
            if name == "dlsym":
                fn(Index(1), "x")
            elif name in ("call_function", "call_cdeclfunction"):
                fn(Index(1), ())
            else:
                fn(Index(1))


class LenIndex:
    def __len__(self):
        return Index(2)


assert len(LenIndex()) == 2

import operator


class HintIndex:
    def __iter__(self):
        return iter(())

    def __length_hint__(self):
        return Index(1)


with assert_raises(TypeError):
    operator.length_hint(HintIndex())
