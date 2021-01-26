
"""create and manipulate C data types in Python"""

import os as _os, sys as _sys

__version__ = "1.1.0"

from _ctypes import Struct, Array
from _ctypes import _Pointer
from _ctypes import CFuncPtr as _CFuncPtr
from _ctypes import dlopen as _dlopen

from struct import calcsize as _calcsize

def create_string_buffer(init, size=None):
    """create_string_buffer(aBytes) -> character array
    create_string_buffer(anInteger) -> character array
    create_string_buffer(aBytes, anInteger) -> character array
    """
    if isinstance(init, bytes):
        if size is None:
            size = len(init)+1
        _sys.audit("ctypes.create_string_buffer", init, size)
        buftype = c_char * size
        buf = buftype()
        buf.value = init
        return buf
    elif isinstance(init, int):
        _sys.audit("ctypes.create_string_buffer", None, init)
        buftype = c_char * init
        buf = buftype()
        return buf
    raise TypeError(init)

def c_buffer(init, size=None):
    return create_string_buffer(init, size)

_c_functype_cache = {}
def CFUNCTYPE(restype, *argtypes, **kw):
    """CFUNCTYPE(restype, *argtypes,
                 use_errno=False, use_last_error=False) -> function prototype.
    restype: the result type
    argtypes: a sequence specifying the argument types
    The function prototype can be called in different ways to create a
    callable object:
    prototype(integer address) -> foreign function
    prototype(callable) -> create and return a C callable function from callable
    prototype(integer index, method name[, paramflags]) -> foreign function calling a COM method
    prototype((ordinal number, dll object)[, paramflags]) -> foreign function exported by ordinal
    prototype((function name, dll object)[, paramflags]) -> foreign function exported by name
    """
    if kw:
        raise ValueError("unexpected keyword argument(s) %s" % kw.keys())
    try:
        return _c_functype_cache[(restype, argtypes)]
    except KeyError:
        class CFunctionType(_CFuncPtr):
            _argtypes_ = argtypes
            _restype_ = restype
        _c_functype_cache[(restype, argtypes)] = CFunctionType
        return CFunctionType

if _os.name == "nt":
    _win_functype_cache = {}
    def WINFUNCTYPE(restype, *argtypes, **kw):
        # docstring set later (very similar to CFUNCTYPE.__doc__)
        if kw:
            raise ValueError("unexpected keyword argument(s) %s" % kw.keys())
        try:
            return _win_functype_cache[(restype, argtypes)]
        except KeyError:
            class WinFunctionType(_CFuncPtr):
                _argtypes_ = argtypes
                _restype_ = restype
            _win_functype_cache[(restype, argtypes)] = WinFunctionType
            return WinFunctionType
    if WINFUNCTYPE.__doc__:
        WINFUNCTYPE.__doc__ = CFUNCTYPE.__doc__.replace("CFUNCTYPE", "WINFUNCTYPE")


from _ctypes import sizeof, byref, addressof, alignment
from _ctypes import _SimpleCData


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
    _check_size(c_ulonglong)

class c_ubyte(_SimpleCData):
    _type_ = "B"
c_ubyte.__ctype_le__ = c_ubyte.__ctype_be__ = c_ubyte
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

from _ctypes import POINTER, pointer, _pointer_type_cache

class c_wchar_p(_SimpleCData):
    _type_ = "Z"
    def __repr__(self):
        return "%s(%s)" % (self.__class__.__name__, c_void_p.from_buffer(self).value)

class c_wchar(_SimpleCData):
    _type_ = "u"

def _reset_cache():
    _pointer_type_cache.clear()
    _c_functype_cache.clear()
    if _os.name == "nt":
        _win_functype_cache.clear()
    # _SimpleCData.c_wchar_p_from_param
    POINTER(c_wchar).from_param = c_wchar_p.from_param
    # _SimpleCData.c_char_p_from_param
    POINTER(c_char).from_param = c_char_p.from_param
    _pointer_type_cache[None] = c_void_p

def create_unicode_buffer(init, size=None):
    """create_unicode_buffer(aString) -> character array
    create_unicode_buffer(anInteger) -> character array
    create_unicode_buffer(aString, anInteger) -> character array
    """
    if isinstance(init, str):
        if size is None:
            if sizeof(c_wchar) == 2:
                # UTF-16 requires a surrogate pair (2 wchar_t) for non-BMP
                # characters (outside [U+0000; U+FFFF] range). +1 for trailing
                # NUL character.
                size = sum(2 if ord(c) > 0xFFFF else 1 for c in init) + 1
            else:
                # 32-bit wchar_t (1 wchar_t per Unicode character). +1 for
                # trailing NUL character.
                size = len(init) + 1
        _sys.audit("ctypes.create_unicode_buffer", init, size)
        buftype = c_wchar * size
        buf = buftype()
        buf.value = init
        return buf
    elif isinstance(init, int):
        _sys.audit("ctypes.create_unicode_buffer", None, init)
        buftype = c_wchar * init
        buf = buftype()
        return buf
    raise TypeError(init)


# XXX Deprecated
def SetPointerType(pointer, cls):
    if _pointer_type_cache.get(cls, None) is not None:
        raise RuntimeError("This type already exists in the cache")
    if id(pointer) not in _pointer_type_cache:
        raise RuntimeError("What's this???")
    pointer.set_type(cls)
    _pointer_type_cache[cls] = pointer
    del _pointer_type_cache[id(pointer)]

# XXX Deprecated
def ARRAY(typ, len):
    return typ * len

################################################################

class CDLL(object):
    """An instance of this class represents a loaded dll/shared
    library, exporting functions using the standard C calling
    convention (named 'cdecl' on Windows).
    The exported functions can be accessed as attributes, or by
    indexing with the function name.  Examples:
    <obj>.qsort -> callable object
    <obj>['qsort'] -> callable object
    """
    _name = '<uninitialized>'
    _handle = 0

    def __init__(self, name,handle=None):
        self._name = name

        class _FuncPtr(_CFuncPtr):
            pass
        
        self._FuncPtr = _FuncPtr

        if handle is None:
            self._handle = _dlopen(self._name)
        else:
            self._handle = handle

    def __repr__(self):
        return "<%s '%s', handle %x at %#x>" % \
               (self.__class__.__name__, self._name,
                (self._handle & (_sys.maxsize*2 + 1)),
                id(self) & (_sys.maxsize*2 + 1))

    def __getattr__(self, name):
        if name.startswith('__') and name.endswith('__'):
            raise AttributeError(name)
        func = self.__getitem__(name)
        setattr(self, name, func)
        return func

    def __getitem__(self, name_or_ordinal):
        func = self._FuncPtr(name_or_ordinal, self)
        if not isinstance(name_or_ordinal, int):
            func.__name__ = name_or_ordinal
        return func

class LibraryLoader(object):
    def __init__(self, dlltype):
        self._dlltype = dlltype

    def __getattr__(self, name):
        if name[0] == '_':
            raise AttributeError(name)

        dll = self._dlltype(name)
        setattr(self, name, dll)

        return dll

    def __getitem__(self, name):
        return getattr(self, name)

    def LoadLibrary(self, name):
        return self._dlltype(name)

cdll = LibraryLoader(CDLL)

if _os.name == "nt":
    windll = LibraryLoader(CDLL)
    oledll = LibraryLoader(CDLL)

    GetLastError = windll.kernel32.GetLastError

    def WinError(code=None, descr=None):
        if code is None:
            code = GetLastError()
        if descr is None:
            # descr = FormatError(code).strip()
            descr = "Windows Error " + str(code)
        return OSError(None, descr, None, code)

if sizeof(c_uint) == sizeof(c_void_p):
    c_size_t = c_uint
    c_ssize_t = c_int
elif sizeof(c_ulong) == sizeof(c_void_p):
    c_size_t = c_ulong
    c_ssize_t = c_long
elif sizeof(c_ulonglong) == sizeof(c_void_p):
    c_size_t = c_ulonglong
    c_ssize_t = c_longlong

# Fill in specifically-sized types
c_int8 = c_byte
c_uint8 = c_ubyte
for kind in [c_short, c_int, c_long, c_longlong]:
    if sizeof(kind) == 2: c_int16 = kind
    elif sizeof(kind) == 4: c_int32 = kind
    elif sizeof(kind) == 8: c_int64 = kind
for kind in [c_ushort, c_uint, c_ulong, c_ulonglong]:
    if sizeof(kind) == 2: c_uint16 = kind
    elif sizeof(kind) == 4: c_uint32 = kind
    elif sizeof(kind) == 8: c_uint64 = kind
del(kind)

# _reset_cache()