import os as _os, sys as _sys

from _ctypes import CFuncPtr as _CFuncPtr
from _ctypes import dlopen as _dlopen
from _ctypes import _SimpleCData

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

class c_short(_SimpleCData):
    _type_ = "h"

class c_ushort(_SimpleCData):
    _type_ = "H"

class c_long(_SimpleCData):
    _type_ = "l"

class c_ulong(_SimpleCData):
    _type_ = "L"

class c_int(_SimpleCData):
    _type_ = "i"

class c_uint(_SimpleCData):
    _type_ = "I"

class c_float(_SimpleCData):
    _type_ = "f"

class c_double(_SimpleCData):
    _type_ = "d"

class c_longdouble(_SimpleCData):
    _type_ = "g"

class c_longlong(_SimpleCData):
    _type_ = "q"

class c_ulonglong(_SimpleCData):
    _type_ = "Q"

class c_ubyte(_SimpleCData):
    _type_ = "B"

class c_byte(_SimpleCData):
    _type_ = "b"

class c_char(_SimpleCData):
    _type_ = "c"

class c_char_p(_SimpleCData):
    _type_ = "z"

class c_void_p(_SimpleCData):
    _type_ = "P"
    
c_voidp = c_void_p # backwards compatibility (to a bug)

class c_bool(_SimpleCData):
    _type_ = "?"

class c_wchar_p(_SimpleCData):
    _type_ = "Z"

class c_wchar(_SimpleCData):
    _type_ = "u"

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
