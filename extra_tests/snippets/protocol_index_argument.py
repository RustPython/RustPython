"""An object with `__index__` is accepted wherever an integer argument is."""

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


# ctypes addresses are the exception: the value is dereferenced as a pointer, so
# CPython takes an exact integer there and never calls `__index__`.
try:
    import ctypes
except ImportError:
    ctypes = None

if ctypes is not None:
    buffer = ctypes.create_string_buffer(8)
    address = ctypes.addressof(buffer)

    assert ctypes.c_int.from_address(address).value == 0

    class Address:
        def __index__(self):
            raise AssertionError("__index__ must not be called for an address")

    assert message(lambda: ctypes.c_int.from_address(Address())) == "integer expected"

    import _ctypes

    assert message(lambda: _ctypes.PyObj_FromPtr(Address())) == "an integer is required"

    # An address is read the way `PyLong_AsVoidPtr` reads one, so either signedness is
    # accepted, and a value that fits neither overflows. The boundaries are pointer-width,
    # so they are derived rather than written out for a 64-bit host.
    pointer_bits = ctypes.sizeof(ctypes.c_void_p) * 8
    unsigned_max = 2**pointer_bits - 1
    signed_min = -(2 ** (pointer_bits - 1))

    assert ctypes.c_int.from_address(unsigned_max - 7) is not None
    assert ctypes.c_int.from_address(-1) is not None

    for out_of_range in (unsigned_max + 1, signed_min - 1):
        try:
            ctypes.c_int.from_address(out_of_range)
        except OverflowError:
            pass
        else:
            raise AssertionError("expected an OverflowError")
