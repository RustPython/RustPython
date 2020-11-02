import os, sys

from _ctypes import CFuncPtr as _CFuncPtr
from _ctypes import dlopen as _dlopen

class CDLL(object):
    """An instance of this class represents a loaded dll/shared
    library, exporting functions using the standard C calling
    convention (named 'cdecl' on Windows).
    The exported functions can be accessed as attributes, or by
    indexing with the function name.  Examples:
    <obj>.qsort -> callable object
    <obj>['qsort'] -> callable object
    Calling the functions releases the Python GIL during the call and
    reacquires it afterwards.
    """
    # default values for repr
    _name = '<uninitialized>'
    _handle = 0
    _FuncPtr = None

    def __init__(self, name, handle=None):
        self._name = name

        class _FuncPtr(_CFuncPtr):
            _restype_ = self._func_restype_

        self._FuncPtr = _FuncPtr

        if handle is None:
            self._handle = _dlopen(self._name)
        else:
            self._handle = handle

    def __repr__(self):
        return "<%s '%s', handle %x at %#x>" % \
               (self.__class__.__name__, self._name,
                (self._handle & (sys.maxsize*2 + 1)),
                id(self) & (sys.maxsize*2 + 1))

    def __getattr__(self, name):
        if name.startswith('__') and name.endswith('__'):
            raise AttributeError(name)

        func = self.__getitem__(name)
        setattr(self, name, func)
        
        return func

    def __getitem__(self, name_or_ordinal):
        func = self._FuncPtr((name_or_ordinal, self))

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