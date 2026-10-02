# Exercises the migrated _ctypes foreign-call path (routed through the unified
# host_env `call` entry point): scalar int/double arguments and returns,
# pointer (c_char_p / c_void_p) returns, a use_errno round-trip, and the
# argument conversion an untyped call performs.
#
# Prints "OK" and exits 0; any failed assertion aborts. Output is identical
# under CPython and RustPython on the same platform.
import ctypes
import errno
import sys
from ctypes import (
    CDLL,
    c_char_p,
    c_double,
    c_int,
    c_long,
    c_size_t,
    c_void_p,
    get_errno,
    set_errno,
)

if sys.platform == "win32":
    # The C library is not reachable as CDLL(None) on Windows; the migrated
    # path is covered there by test_ctypes. Keep output identical regardless.
    print("OK")
    sys.exit(0)

libc = CDLL(None, use_errno=True)

# 1. scalar int argument + int return: abs(-5) == 5
libc.abs.argtypes = [c_int]
libc.abs.restype = c_int
assert libc.abs(-5) == 5, libc.abs(-5)

# 2. pointer argument (bytes -> char*) + size_t return: strlen(b"hello") == 5
libc.strlen.argtypes = [c_char_p]
libc.strlen.restype = c_size_t
assert libc.strlen(b"hello") == 5, libc.strlen(b"hello")

# 3. double argument + double return: sqrt(2.0)
libc.sqrt.argtypes = [c_double]
libc.sqrt.restype = c_double
root = libc.sqrt(2.0)
assert abs(root - 2.0**0.5) < 1e-12, root

# 4. c_char_p return: strchr(b"abcdef", 'c') -> b"cdef"
libc.strchr.argtypes = [c_char_p, c_int]
libc.strchr.restype = c_char_p
assert libc.strchr(b"abcdef", ord("c")) == b"cdef", libc.strchr(b"abcdef", ord("c"))

# 5. c_void_p return: the same call yields a non-null integer address
libc.strchr.restype = c_void_p
addr = libc.strchr(b"abcdef", ord("c"))
assert isinstance(addr, int) and addr != 0, addr

# 6. use_errno round-trip: strtol overflow sets errno == ERANGE, captured into
#    the ctypes-private errno by the call's errno swap.
libc.strtol.argtypes = [c_char_p, c_void_p, c_int]
libc.strtol.restype = c_long
set_errno(0)
libc.strtol(b"9" * 40, None, 10)
assert get_errno() == errno.ERANGE, (get_errno(), errno.ERANGE)

# 7. A float has no implicit conversion to an integer argument: converting it
#    would pass a truncated value where the callee expects an int or a pointer.
libc.abs.argtypes = None
for bad in (1.5, 0.0, 1e300):
    try:
        libc.abs(bad)
    except (TypeError, ctypes.ArgumentError):
        pass
    else:
        assert False, f"{bad!r} was accepted as an integer argument"
assert libc.abs(-3) == 3
assert libc.abs(True) == 1


# 8. Custom argtypes converter with from_param (issue #8341 / rich._win32_console pattern)
class CustomIntAdapter:
    @classmethod
    def from_param(cls, val):
        if isinstance(val, str):
            return int(val)
        elif isinstance(val, tuple):
            return val[0]
        raise ValueError(f"cannot adapt {val!r}")


libc.abs.argtypes = [CustomIntAdapter]
assert libc.abs("-42") == 42
assert libc.abs((-99, "extra")) == 99

try:
    libc.abs(3.14)
except ctypes.ArgumentError as e:
    assert "argument 1" in str(e)
    assert "ValueError: cannot adapt 3.14" in str(e)
else:
    assert False, "expected ArgumentError for failed custom from_param"


# 9. Validation of items in argtypes (must have from_param)
class NoFromParam:
    pass


try:
    libc.abs.argtypes = [NoFromParam]
except TypeError as e:
    assert "has no from_param method" in str(e)
else:
    assert False, "expected TypeError when argtypes item has no from_param method"

try:
    libc.abs.argtypes = [c_int, 42]
except TypeError as e:
    assert "item 2 in _argtypes_ has no from_param method" in str(e)
else:
    assert False, "expected TypeError when item 2 in argtypes has no from_param method"

print("OK")
