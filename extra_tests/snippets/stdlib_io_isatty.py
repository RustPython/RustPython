"""
Test that IOBase.isatty() raises ValueError when called on a closed file.

CPython 3.14 reference: Modules/_io/iobase.c iobase_check_closed() is called
before returning False from _io__IOBase_isatty_impl().

The existing test_io.test_io_after_close does NOT cover this because it uses
concrete classes (TextIOWrapper, BufferedWriter, etc.) that override isatty()
with their own closed checks, never reaching IOBase.isatty() directly.
"""

import io


# Minimal subclass that inherits IOBase.isatty() without overriding it.
class MinimalRaw(io.RawIOBase):
    def readinto(self, b):
        return 0


f = MinimalRaw()
assert not f.closed
assert f.isatty() == False  # open file: should return False

f.close()
assert f.closed

try:
    f.isatty()
    assert False, "ValueError not raised on closed IOBase"
except ValueError:
    pass  # expected
