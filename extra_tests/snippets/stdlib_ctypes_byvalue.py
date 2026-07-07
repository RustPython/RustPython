# ctypes by-value aggregate arguments and returns over the live FFI path.
#
# Exercises passing structs/unions BY VALUE to foreign functions and returning
# structs BY VALUE through the unified host_env `call` entry point:
#   - div(7, 3) / div(-7, 3): return div_t{quot, rem} by value (8-byte int
#     struct, register-returned on SysV/AArch64),
#   - imaxdiv(7, 3): return imaxdiv_t{quot, rem} by value (16-byte two-long
#     struct, two-register return on SysV),
#   - inet_ntoa(struct in_addr): take a 4-byte struct by value, with argtypes,
#     without argtypes (direct-instance paramfunc path), and via a Union.
#
# Runs on little-endian linux/macOS; skipped on Windows (see below). Prints
# "OK"; a failed assertion aborts with a non-zero status.

import ctypes
import sys
from ctypes import (
    CDLL,
    Structure,
    Union,
    c_char,
    c_char_p,
    c_int,
    c_int64,
    c_uint32,
    sizeof,
)

if sys.platform == "win32":
    # The C library is not reachable as CDLL(None) on Windows; by-value
    # aggregate calls are covered there by test_ctypes. Keep output identical.
    print("OK")
    sys.exit(0)


libc = CDLL(None)


# 1. struct RETURN by value: div(7, 3) -> div_t{quot=2, rem=1}
class div_t(Structure):
    _fields_ = [("quot", c_int), ("rem", c_int)]


assert sizeof(div_t) == 8, sizeof(div_t)
libc.div.argtypes = [c_int, c_int]
libc.div.restype = div_t

r = libc.div(7, 3)
assert isinstance(r, div_t)
assert (r.quot, r.rem) == (2, 1), (r.quot, r.rem)

# C division truncates toward zero.
r = libc.div(-7, 3)
assert (r.quot, r.rem) == (-2, -1), (r.quot, r.rem)

# struct RETURN by value with NO argtypes on the arguments (ints via ConvParam)
libc.div.argtypes = None
r = libc.div(17, 5)
assert (r.quot, r.rem) == (3, 2), (r.quot, r.rem)


# 2. larger struct RETURN by value: imaxdiv(7, 3) -> imaxdiv_t{quot=2, rem=1}
class imaxdiv_t(Structure):
    _fields_ = [("quot", c_int64), ("rem", c_int64)]


assert sizeof(imaxdiv_t) == 16, sizeof(imaxdiv_t)
libc.imaxdiv.argtypes = [c_int64, c_int64]
libc.imaxdiv.restype = imaxdiv_t

r = libc.imaxdiv(7, 3)
assert (r.quot, r.rem) == (2, 1), (r.quot, r.rem)
r = libc.imaxdiv(-9, 4)
assert (r.quot, r.rem) == (-2, -1), (r.quot, r.rem)


# 3. struct ARGUMENT by value: inet_ntoa(struct in_addr) -> b"1.2.3.4"
class in_addr(Structure):
    _fields_ = [("s_addr", c_uint32)]


assert sizeof(in_addr) == 4, sizeof(in_addr)
# `s_addr` holds the four address bytes in memory (network) order; a host-endian
# int whose bytes are [1, 2, 3, 4] yields the dotted string "1.2.3.4".
addr_value = int.from_bytes(bytes([1, 2, 3, 4]), sys.byteorder)

libc.inet_ntoa.argtypes = [in_addr]
libc.inet_ntoa.restype = c_char_p
assert libc.inet_ntoa(in_addr(addr_value)) == b"1.2.3.4"

# struct ARGUMENT by value with NO argtypes (direct-instance paramfunc path)
libc.inet_ntoa.argtypes = None
assert libc.inet_ntoa(in_addr(addr_value)) == b"1.2.3.4"


# 4. union ARGUMENT by value: a union laid out like in_addr, passed by value.
class in_addr_u(Union):
    _fields_ = [("s_addr", c_uint32), ("bytes", c_char * 4)]


assert sizeof(in_addr_u) == 4, sizeof(in_addr_u)
libc.inet_ntoa.argtypes = [in_addr_u]
libc.inet_ntoa.restype = c_char_p
assert libc.inet_ntoa(in_addr_u(addr_value)) == b"1.2.3.4"

print("OK")
