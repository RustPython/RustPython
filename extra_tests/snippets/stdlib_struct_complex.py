import struct
import sys
import warnings

from testutils import assert_raises


class ComplexLike:
    def __complex__(self):
        return 1 + 2j


class ComplexSubclass(complex):
    def __complex__(self):
        raise AssertionError("complex subclasses use their stored value")


with warnings.catch_warnings():
    warnings.simplefilter("error")
    for code in ("F", "D"):
        expected = struct.pack(code, 1 + 2j)
        assert struct.pack(code, ComplexLike()) == expected
        assert struct.pack(code, ComplexSubclass(1, 2)) == expected

native = "<" if sys.byteorder == "little" else ">"
swapped = ">" if sys.byteorder == "little" else "<"
for value in (complex(1e300, 0), complex(0, 1e300)):
    assert struct.pack(native + "F", value) == struct.pack("@F", value)
    with assert_raises(OverflowError):
        struct.pack(swapped + "F", value)
