"""Focused native prerequisites for the pinned CPython 3.15 rc2 stdlib."""

import _codecs
import array
import binascii
import encodings
import sys
import warnings


def raises(kind, call, *args, **kwargs):
    try:
        call(*args, **kwargs)
    except kind as exc:
        return exc
    raise AssertionError(f"Expected {kind.__name__}")


normalize = _codecs._normalize_encoding
for original, expected in [
    ("", ""),
    ("UTF 8", "UTF_8"),
    ("utf_8", "utf_8"),
    ("utf   8", "utf_8"),
    (" -_A--B_- ", "A_B"),
    ("utf...8", "utf...8"),
    (".A-.", ".A_."),
    ("AéB", "A_B"),
    ("Aé-€B", "A_B"),
    ("é€", ""),
    ("éA€", "A"),
    ("utfé€\U0010ffff-8", "utf_8"),
    ("A\x00B", "A"),
    ("\x00A", ""),
    ("A-\x00B", "A"),
]:
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        actual = normalize(original)
    assert actual == expected, (original, actual, expected)
    assert type(actual) is str
    assert not caught

assert normalize(encoding="UTF-8") == "UTF_8"
for invalid in (b"UTF 8", bytearray(b"UTF 8"), 42, None):
    raises(TypeError, normalize, invalid)
raises(TypeError, normalize)
raises(TypeError, normalize, "a", encoding="b")


class StrSubclass(str):
    def __str__(self):
        raise AssertionError("native normalization must use string storage")


assert normalize(StrSubclass("UTF-8")) == "UTF_8"
for original, start, end in [
    ("A\ud800B", 1, 2),
    ("A\ud800\udfffB", 1, 3),
    ("A\x00\ud800", 2, 3),
    ("éA\ud800B", 2, 3),
    ("\U0001f600\ud800\udfffB\ud900", 1, 3),
]:
    exc = raises(UnicodeEncodeError, normalize, original)
    assert (exc.encoding, exc.object, exc.start, exc.end, exc.reason) == (
        "utf-8",
        original,
        start,
        end,
        "surrogates not allowed",
    )

assert encodings.normalize_encoding(b"UTF 8") == "UTF_8"
with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    assert encodings.normalize_encoding("utfé-8") == "utf_8"
assert len(caught) == 1
assert caught[0].category is DeprecationWarning
assert (
    str(caught[0].message)
    == "Support for non-ascii encoding names will be removed in 3.17"
)

encode = binascii.b2a_base64
for make in (bytes, bytearray, memoryview, lambda b: array.array("B", b)):
    data = make(b"www.python.org")
    expected = {
        0: b"d3d3LnB5dGhvbi5vcmc=",
        1: b"d3d3\nLnB5\ndGhv\nbi5v\ncmc=",
        2: b"d3d3\nLnB5\ndGhv\nbi5v\ncmc=",
        3: b"d3d3\nLnB5\ndGhv\nbi5v\ncmc=",
        4: b"d3d3\nLnB5\ndGhv\nbi5v\ncmc=",
        7: b"d3d3\nLnB5\ndGhv\nbi5v\ncmc=",
        8: b"d3d3LnB5\ndGhvbi5v\ncmc=",
        11: b"d3d3LnB5\ndGhvbi5v\ncmc=",
        12: b"d3d3LnB5dGhv\nbi5vcmc=",
        19: b"d3d3LnB5dGhvbi5v\ncmc=",
        20: b"d3d3LnB5dGhvbi5vcmc=",
        sys.maxsize: b"d3d3LnB5dGhvbi5vcmc=",
        2 * sys.maxsize + 1: b"d3d3LnB5dGhvbi5vcmc=",
    }
    for width, result in expected.items():
        assert encode(data, wrapcol=width, newline=False) == result
        assert encode(data, wrapcol=width) == result + b"\n"
    for width in (0, 1, 4, 8, sys.maxsize, 2 * sys.maxsize + 1):
        assert encode(make(b""), wrapcol=width, newline=False) == b""
        assert encode(make(b""), wrapcol=width) == b"\n"
    assert encode(make(b"a"), wrapcol=1) == b"YQ==\n"
    assert encode(make(b"ab"), wrapcol=1, newline=False) == b"YWI="
    assert encode(make(b"abc"), wrapcol=4) == b"YWJj\n"
    assert encode(make(b"abcdef"), wrapcol=4) == b"YWJj\nZGVm\n"
    assert encode(make(b"abcdef"), wrapcol=4, newline=False) == b"YWJj\nZGVm"


class Index:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


class IntOnly:
    def __int__(self):
        return 8


class IntSubclass(int):
    def __index__(self):
        raise AssertionError("int subclass must use stored value")


assert encode(b"abcdef", wrapcol=Index(4)) == b"YWJj\nZGVm\n"
assert encode(b"abcdef", wrapcol=IntSubclass(4)) == b"YWJj\nZGVm\n"
assert encode(b"abcdef", wrapcol=True) == b"YWJj\nZGVm\n"
assert encode(b"abcdef", wrapcol=False) == b"YWJjZGVm\n"
for data in (b"", b"abc"):
    for invalid in (-1, -(2**1000), Index(-1)):
        raises(ValueError, encode, data, wrapcol=invalid)
    for invalid in (2 * sys.maxsize + 2, 2**1000, Index(2**1000)):
        raises(OverflowError, encode, data, wrapcol=invalid)
    for invalid in (8.0, "8", None, IntOnly(), Index(8.0)):
        raises(TypeError, encode, data, wrapcol=invalid)
for truth in (1, -1, [1]):
    assert encode(b"abcdef", wrapcol=4, newline=truth) == b"YWJj\nZGVm\n"
for false in (0, None, []):
    assert encode(b"abcdef", wrapcol=4, newline=false) == b"YWJj\nZGVm"


class TruthFailure:
    def __bool__(self):
        raise RuntimeError("truth failure")


raises(RuntimeError, encode, b"", newline=TruthFailure())
raises(ValueError, encode, b"", wrapcol=-1, newline=TruthFailure())
raises(TypeError, encode, data=b"abc")
raises(TypeError, encode, b"abc", 8)
raises(TypeError, encode, "bad", wrapcol=-1)
raises(BufferError, encode, memoryview(b"abcdef")[::2], wrapcol=4)

print("native codec prerequisites: passed")
