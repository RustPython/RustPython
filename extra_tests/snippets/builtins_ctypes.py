from _ctypes import sizeof
from _ctypes import _SimpleCData
from struct import calcsize as _calcsize

def _check_size(typ, typecode=None):
    # Check if sizeof(ctypes_type) against struct.calcsize.  This
    # should protect somewhat against a misconfigured libffi.
    from struct import calcsize
    if typecode is None:
        # Most _type_ codes are the same as used in struct
        typecode = typ._type_
    actual, required = sizeof(typ), calcsize(typecode)
    if actual != required:
        raise SystemError("sizeof(%s) wrong: %d instead of %d" % \
                          (typ, actual, required))

class c_short(_SimpleCData):
    _type_ = "h"
_check_size(c_short)

class c_ushort(_SimpleCData):
    _type_ = "H"
_check_size(c_ushort)

class c_long(_SimpleCData):
    _type_ = "l"
_check_size(c_long)

class c_ulong(_SimpleCData):
    _type_ = "L"
_check_size(c_ulong)

if _calcsize("i") == _calcsize("l"):
    # if int and long have the same size, make c_int an alias for c_long
    c_int = c_long
    c_uint = c_ulong
else:
    class c_int(_SimpleCData):
        _type_ = "i"
    _check_size(c_int)

    class c_uint(_SimpleCData):
        _type_ = "I"
    _check_size(c_uint)

class c_float(_SimpleCData):
    _type_ = "f"
_check_size(c_float)

class c_double(_SimpleCData):
    _type_ = "d"
_check_size(c_double)

class c_longdouble(_SimpleCData):
    _type_ = "g"
if sizeof(c_longdouble) == sizeof(c_double):
    c_longdouble = c_double

if _calcsize("l") == _calcsize("q"):
    # if long and long long have the same size, make c_longlong an alias for c_long
    c_longlong = c_long
    c_ulonglong = c_ulong
else:
    class c_longlong(_SimpleCData):
        _type_ = "q"
    _check_size(c_longlong)

    class c_ulonglong(_SimpleCData):
        _type_ = "Q"
    ##    def from_param(cls, val):
    ##        return ('d', float(val), val)
    ##    from_param = classmethod(from_param)
    _check_size(c_ulonglong)

class c_ubyte(_SimpleCData):
    _type_ = "B"
c_ubyte.__ctype_le__ = c_ubyte.__ctype_be__ = c_ubyte
# backward compatibility:
##c_uchar = c_ubyte
_check_size(c_ubyte)

class c_byte(_SimpleCData):
    _type_ = "b"
c_byte.__ctype_le__ = c_byte.__ctype_be__ = c_byte
_check_size(c_byte)

class c_char(_SimpleCData):
    _type_ = "c"
c_char.__ctype_le__ = c_char.__ctype_be__ = c_char
_check_size(c_char)

class c_char_p(_SimpleCData):
    _type_ = "z"
    def __repr__(self):
        return "%s(%s)" % (self.__class__.__name__, c_void_p.from_buffer(self).value)
_check_size(c_char_p, "P")

class c_void_p(_SimpleCData):
    _type_ = "P"
c_voidp = c_void_p # backwards compatibility (to a bug)
_check_size(c_void_p)

class c_bool(_SimpleCData):
    _type_ = "?"
_check_size(c_bool)
