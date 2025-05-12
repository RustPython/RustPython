import os as _os, sys as _sys
import types as _types

from _ctypes import RTLD_LOCAL, RTLD_GLOBAL
from _ctypes import sizeof
from _ctypes import _SimpleCData, Array
from _ctypes import CFuncPtr as _CFuncPtr

from struct import calcsize as _calcsize


assert Array.__class__.__name__ == "PyCArrayType"
assert Array.__base__.__name__ == "_CData"

DEFAULT_MODE = RTLD_LOCAL
if _os.name == "posix" and _sys.platform == "darwin":
    # On OS X 10.3, we use RTLD_GLOBAL as default mode
    # because RTLD_LOCAL does not work at least on some
    # libraries.  OS X 10.3 is Darwin 7, so we check for
    # that.

    if int(_os.uname().release.split(".")[0]) < 8:
        DEFAULT_MODE = RTLD_GLOBAL

from _ctypes import (
    FUNCFLAG_CDECL as _FUNCFLAG_CDECL,
    FUNCFLAG_PYTHONAPI as _FUNCFLAG_PYTHONAPI,
    FUNCFLAG_USE_ERRNO as _FUNCFLAG_USE_ERRNO,
    FUNCFLAG_USE_LASTERROR as _FUNCFLAG_USE_LASTERROR,
)


def create_string_buffer(init, size=None):
    """create_string_buffer(aBytes) -> character array
    create_string_buffer(anInteger) -> character array
    create_string_buffer(aBytes, anInteger) -> character array
    """
    if isinstance(init, bytes):
        if size is None:
            size = len(init) + 1
        _sys.audit("ctypes.create_string_buffer", init, size)
        buftype = c_char.__mul__(size)
        print(type(c_char.__mul__(size)))
        # buftype = c_char * size
        buf = buftype()
        buf.value = init
        return buf
    elif isinstance(init, int):
        _sys.audit("ctypes.create_string_buffer", None, init)
        buftype = c_char.__mul__(init)
        # buftype = c_char * init
        buf = buftype()
        return buf
    raise TypeError(init)


def _check_size(typ, typecode=None):
    # Check if sizeof(ctypes_type) against struct.calcsize.  This
    # should protect somewhat against a misconfigured libffi.
    from struct import calcsize

    if typecode is None:
        # Most _type_ codes are the same as used in struct
        typecode = typ._type_
    actual, required = sizeof(typ), calcsize(typecode)
    if actual != required:
        raise SystemError(
            "sizeof(%s) wrong: %d instead of %d" % (typ, actual, required)
        )


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


c_voidp = c_void_p  # backwards compatibility (to a bug)
_check_size(c_void_p)


class c_bool(_SimpleCData):
    _type_ = "?"


_check_size(c_bool)

i = c_int(42)
f = c_float(3.14)
# s = create_string_buffer(b'\000' * 32)
assert i.value == 42
assert abs(f.value - 3.14) < 1e-06

if _os.name == "nt":
    from _ctypes import LoadLibrary as _dlopen
    from _ctypes import FUNCFLAG_STDCALL as _FUNCFLAG_STDCALL
elif _os.name == "posix":
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

    _func_flags_ = _FUNCFLAG_CDECL
    _func_restype_ = c_int
    # default values for repr
    _name = "<uninitialized>"
    _handle = 0
    _FuncPtr = None

    def __init__(
        self,
        name,
        mode=DEFAULT_MODE,
        handle=None,
        use_errno=False,
        use_last_error=False,
        winmode=None,
    ):
        self._name = name
        flags = self._func_flags_
        if use_errno:
            flags |= _FUNCFLAG_USE_ERRNO
        if use_last_error:
            flags |= _FUNCFLAG_USE_LASTERROR
        if _sys.platform.startswith("aix"):
            """When the name contains ".a(" and ends with ")",
               e.g., "libFOO.a(libFOO.so)" - this is taken to be an
               archive(member) syntax for dlopen(), and the mode is adjusted.
               Otherwise, name is presented to dlopen() as a file argument.
            """
            if name and name.endswith(")") and ".a(" in name:
                mode |= _os.RTLD_MEMBER | _os.RTLD_NOW
        if _os.name == "nt":
            if winmode is not None:
                mode = winmode
            else:
                import nt

                mode = 4096
                if "/" in name or "\\" in name:
                    self._name = nt._getfullpathname(self._name)
                    mode |= nt._LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR

        class _FuncPtr(_CFuncPtr):
            _flags_ = flags
            _restype_ = self._func_restype_

        self._FuncPtr = _FuncPtr

        if handle is None:
            self._handle = _dlopen(self._name, mode)
        else:
            self._handle = handle

    def __repr__(self):
        return "<%s '%s', handle %x at %#x>" % (
            self.__class__.__name__,
            self._name,
            (self._handle & (_sys.maxsize * 2 + 1)),
            id(self) & (_sys.maxsize * 2 + 1),
        )

    def __getattr__(self, name):
        if name.startswith("__") and name.endswith("__"):
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
        if name[0] == "_":
            raise AttributeError(name)
        try:
            dll = self._dlltype(name)
        except OSError:
            raise AttributeError(name)
        setattr(self, name, dll)
        return dll

    def __getitem__(self, name):
        return getattr(self, name)

    def LoadLibrary(self, name):
        return self._dlltype(name)

    __class_getitem__ = classmethod(_types.GenericAlias)


cdll = LibraryLoader(CDLL)

test_byte_array = create_string_buffer(b"Hello, World!\n")
assert test_byte_array._length_ == 15

if _os.name == "posix" or _sys.platform == "darwin":
    pass
else:
    import os

    libc = cdll.msvcrt
    libc.rand()
    i = c_int(1)
    print("start srand")
    print(libc.srand(i))
    print(test_byte_array)
    print(test_byte_array._type_)
    # print("start printf")
    # libc.printf(test_byte_array)

    # windows pip support

    def get_win_folder_via_ctypes(csidl_name: str) -> str:
        """Get folder with ctypes."""
        # There is no 'CSIDL_DOWNLOADS'.
        # Use 'CSIDL_PROFILE' (40) and append the default folder 'Downloads' instead.
        # https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid

        import ctypes  # noqa: PLC0415

        csidl_const = {
            "CSIDL_APPDATA": 26,
            "CSIDL_COMMON_APPDATA": 35,
            "CSIDL_LOCAL_APPDATA": 28,
            "CSIDL_PERSONAL": 5,
            "CSIDL_MYPICTURES": 39,
            "CSIDL_MYVIDEO": 14,
            "CSIDL_MYMUSIC": 13,
            "CSIDL_DOWNLOADS": 40,
            "CSIDL_DESKTOPDIRECTORY": 16,
        }.get(csidl_name)
        if csidl_const is None:
            msg = f"Unknown CSIDL name: {csidl_name}"
            raise ValueError(msg)

        buf = ctypes.create_unicode_buffer(1024)
        windll = getattr(ctypes, "windll")  # noqa: B009 # using getattr to avoid false positive with mypy type checker
        windll.shell32.SHGetFolderPathW(None, csidl_const, None, 0, buf)

        # Downgrade to short path name if it has high-bit chars.
        if any(ord(c) > 255 for c in buf):  # noqa: PLR2004
            buf2 = ctypes.create_unicode_buffer(1024)
            if windll.kernel32.GetShortPathNameW(buf.value, buf2, 1024):
                buf = buf2

        if csidl_name == "CSIDL_DOWNLOADS":
            return os.path.join(buf.value, "Downloads")  # noqa: PTH118

        return buf.value

    # print(get_win_folder_via_ctypes("CSIDL_DOWNLOADS"))
