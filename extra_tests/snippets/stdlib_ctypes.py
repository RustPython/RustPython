import ctypes
import gc
import itertools
import os as _os
import sys as _sys
import types as _types
import weakref
from _ctypes import RTLD_GLOBAL, RTLD_LOCAL, Array, _SimpleCData, sizeof
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
)
from _ctypes import (
    FUNCFLAG_PYTHONAPI as _FUNCFLAG_PYTHONAPI,
)
from _ctypes import (
    FUNCFLAG_USE_ERRNO as _FUNCFLAG_USE_ERRNO,
)
from _ctypes import (
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

char_pointer = c_char_p(b"1.3.6.1.5.5.7.3.1")
char_pointer_array = (c_char_p * 1)(char_pointer)
assert char_pointer_array[0] == b"1.3.6.1.5.5.7.3.1"


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
    from _ctypes import FUNCFLAG_STDCALL as _FUNCFLAG_STDCALL
    from _ctypes import LoadLibrary as _dlopen
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

if _os.name == "posix":
    if _sys.platform == "darwin":
        libc = cdll.LoadLibrary("libc.dylib")
        libc.rand()
        i = c_int(1)
        # print("start srand")
        # print(libc.srand(i))
        # print(test_byte_array)
else:
    import ctypes
    import os
    from ctypes import wintypes

    libc = cdll.msvcrt
    libc.rand()
    i = c_int(1)
    print("start srand")
    # print(libc.srand(i))
    # print(test_byte_array)
    # print(test_byte_array._type_)
    # print("start printf")
    # libc.printf(test_byte_array)

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    get_current_process = kernel32.GetCurrentProcess
    get_current_process.argtypes = ()
    get_current_process.restype = ctypes.c_void_p

    def preserve_result(_result, _func, args):
        return args

    get_current_process.errcheck = preserve_result
    process_handle = get_current_process()
    assert isinstance(process_handle, int)

    get_process_id = kernel32.GetProcessId
    get_process_id.argtypes = (ctypes.c_void_p,)
    get_process_id.restype = wintypes.DWORD
    assert get_process_id(process_handle) == os.getpid()

    def replace_result(_result, _func, _args):
        return "replacement"

    get_current_process.errcheck = replace_result
    assert get_current_process() == "replacement"

    # windows pip support

    def get_win_folder_via_ctypes(csidl_name: str) -> str:
        """Get folder with ctypes."""
        # There is no 'CSIDL_DOWNLOADS'.
        # Use 'CSIDL_PROFILE' (40) and append the default folder 'Downloads' instead.
        # https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid

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

# A value wider than the C type is masked down to it instead of failing an
# unchecked conversion.
assert ctypes.c_char_p(2**64).value is None
assert ctypes.c_int(2**64 + 7).value == 7
buf = (ctypes.c_int * 1)()
int_ptr = ctypes.cast(buf, ctypes.POINTER(ctypes.c_int))
int_ptr[0] = 2**64 + 5
assert int_ptr[0] == 5

# A slice assignment is length-checked against the slice, so the right-hand
# side must not be drained first.
array3 = (ctypes.c_int * 3)()
try:
    array3[0:3] = itertools.count()
except ValueError:
    pass
else:
    raise AssertionError("slice assignment accepted an unbounded iterable")
array3[0:3] = [7, 8, 9]
assert list(array3) == [7, 8, 9]


# An array type carries the size of its buffer, so one too large to allocate
# must raise instead of aborting.
try:
    (ctypes.c_char * (2**60))()
except MemoryError:
    pass
else:
    raise AssertionError("an unallocatable array was created")

# A function-pointer cast retains the callback and its executable address.
Unary = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int)
callback = Unary(lambda value: value + 1)
address = ctypes.cast(callback, ctypes.c_void_p)
roundtrip = ctypes.cast(address, Unary)
assert roundtrip._objects[id(callback)] is callback
del callback, address
gc.collect()
assert roundtrip(41) == 42


# All ctypes metaclasses use sequence repetition in either direction.
class RepeatStruct(ctypes.Structure):
    _fields_ = [("value", ctypes.c_int)]


class RepeatUnion(ctypes.Union):
    _fields_ = [("value", ctypes.c_int)]


for element_type in (
    ctypes.c_int,
    ctypes.c_int * 1,
    ctypes.POINTER(ctypes.c_int),
    RepeatStruct,
    RepeatUnion,
    Unary,
):
    assert 2 * element_type is element_type * 2


class ArrayLength:
    def __index__(self):
        return 2


assert ArrayLength() * ctypes.c_int is ctypes.c_int * 2


class ReflectedLength(ArrayLength):
    def __rmul__(self, other):
        return "reflected"


assert ctypes.c_int * ReflectedLength() == "reflected"


class ArrayField(ctypes.Structure):
    _fields_ = [("prefix", ctypes.c_int), ("values", ctypes.c_int * 3)]


owner = ArrayField(99, (1, 2, 3))
view = owner.values
assert view._b_base_ is owner
assert ctypes.addressof(view) == ctypes.addressof(owner) + ArrayField.values.offset
ctypes.resize(owner, ctypes.sizeof(owner))
assert ctypes.addressof(view) == ctypes.addressof(owner) + ArrayField.values.offset
pointer = ctypes.cast(view, ctypes.POINTER(ctypes.c_int))
pointer[1] = 42
assert list(owner.values) == [1, 42, 3]
owner.values[0] = 7
assert bytes(view) == bytes(owner.values)
memoryview(view).cast("B")[:] = bytes((ctypes.c_int * 3)(7, 42, 9))
assert list(owner.values) == [7, 42, 9]
del owner
assert list(view) == [7, 42, 9]


class OverlappingFields(ctypes.Union):
    _fields_ = [("a", ctypes.c_ubyte * 8), ("b", ctypes.c_ubyte * 8)]


overlap = OverlappingFields()
overlap.a[:] = range(8)
memoryview(overlap.a).cast("B")[::2] = memoryview(overlap.b).cast("B")[:4]
assert list(overlap.a) == [0, 1, 1, 3, 2, 5, 3, 7]

backing = bytearray(range(8))
view = (ctypes.c_ubyte * 7).from_buffer(backing, 1)
view._objects.clear()  # Alias detection must not depend on mutable retention metadata.
memoryview(view).cast("B")[::2] = memoryview(backing)[:4]
assert list(backing) == [0, 0, 2, 1, 4, 2, 6, 3]

raw_owner = (ctypes.c_ubyte * 8)()
raw_views = [type(raw_owner).from_address(ctypes.addressof(raw_owner))]
if _sys.version_info >= (3, 14):
    raw_views.append(
        ctypes.memoryview_at(ctypes.addressof(raw_owner), ctypes.sizeof(raw_owner))
    )
for raw_view in raw_views:
    for source, dest in ((raw_owner, raw_view), (raw_view, raw_owner)):
        raw_owner[:] = range(8)
        memoryview(dest).cast("B")[::2] = memoryview(source).cast("B")[:4]
        assert list(raw_owner) == [0, 1, 1, 3, 2, 5, 3, 7]


# A snapshotted source's release hook must run after the destination write.
raw_owner[:] = range(8)
destination_buffer = bytearray(b"abcdefgh")
destination_view = memoryview(destination_buffer)
release_events = []


class ReleasingBuffer:
    def __buffer__(self, flags):
        return memoryview(raw_views[0]).cast("B")

    def __release_buffer__(self, view):
        release_events.append(bytes(destination_buffer))
        destination_view.release()
        destination_buffer.clear()


destination_view[:] = ReleasingBuffer()
assert release_events == [bytes(raw_owner)]
assert destination_buffer == b""


# byref carries an address, including its byte offset, rather than the value
# stored in the referenced object.
pair = (ctypes.c_int * 2)(42, 99)
for offset in (0, ctypes.sizeof(ctypes.c_int)):
    ref = ctypes.byref(pair, offset)
    assert ctypes.cast(ref, ctypes.c_void_p).value == ctypes.addressof(pair) + offset
    assert (
        ctypes.string_at(ref, ctypes.sizeof(ctypes.c_int))
        == bytes(pair)[offset : offset + ctypes.sizeof(ctypes.c_int)]
    )
zero = ctypes.c_size_t(0)
assert ctypes.string_at(ctypes.byref(zero), ctypes.sizeof(zero)) == bytes(zero)
param = ctypes.c_void_p.from_param(ctypes.addressof(zero))
assert ctypes.cast(param, ctypes.c_void_p).value == ctypes.addressof(zero)
assert ctypes.string_at(param, ctypes.sizeof(zero)) == bytes(zero)
for read in (
    lambda p: ctypes.cast(p, ctypes.c_void_p),
    lambda p: ctypes.string_at(p, 0),
):
    try:
        read(ctypes.c_char_p.from_param(b"x"))
    except (TypeError, ctypes.ArgumentError):
        pass
    else:
        raise AssertionError("non-pointer argument wrapper accepted")

print("done")


# Indexed compound objects are views of the element, not decoded scalars.
class IndexedPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int)]


class IndexedInt(ctypes.c_int):
    pass


for _element in (IndexedPoint, IndexedInt, ctypes.c_int * 2):
    _owner = (_element * 2)()
    _view = _owner[1]
    assert type(_view) is _element
    assert _view._b_base_ is _owner
    assert ctypes.addressof(_view) == ctypes.addressof(_owner) + ctypes.sizeof(_element)
    _pointer = ctypes.cast(_owner, ctypes.POINTER(_element))
    _pointer[1] = _pointer[1]
    assert bytes(_pointer[1]) == bytes(_view)

_array = (ctypes.c_int * 2)(1, 2)
_pointer = ctypes.pointer(_array)
for _value, _view in enumerate((_pointer[0], _pointer.contents), 42):
    assert _view._b_base_ is _pointer
    assert len(_view) == 2
    _view[1] = _value
    assert _array[1] == _value

_callback_type = ctypes.CFUNCTYPE(ctypes.c_int)
_callbacks = (_callback_type * 1)(_callback_type(lambda: 42))
_callback = _callbacks[0]
assert _callback._b_base_ is _callbacks
_copied_callbacks = (_callback_type * 1)(_callback)
del _callback, _callbacks
gc.collect()
assert _copied_callbacks[0]() == 42


class CallbackField(ctypes.Structure):
    _fields_ = [("callback", _callback_type)]


assert CallbackField(_copied_callbacks[0]).callback() == 42

# Nested element keys and copies must retain both pointees after the source dies.
_nested = (((ctypes.POINTER(ctypes.c_int) * 1) * 1) * 2)()
_refs = []
for _index in range(2):
    _item = ctypes.c_int(_index)
    _refs.append(weakref.ref(_item))
    _nested[_index][0][0] = ctypes.pointer(_item)
_copy = type(_nested)(*_nested)
del _nested, _item
gc.collect()
assert [ref().value for ref in _refs] == [0, 1]

# Replacing a pointer view's contents updates its slot and retains the pointee.
_replacement = ctypes.c_int(42)
_replacement_ref = weakref.ref(_replacement)
_view = _copy[1][0][0]
_view.contents = _replacement
del _replacement, _view
gc.collect()
assert _replacement_ref() is not None
assert _copy[1][0][0].contents.value == 42

# Replacing a callback may reenter the same array from its finalizer.
_finalized = []


class ReenteringCallback:
    def __call__(self):
        return 1

    def __del__(self):
        _finalized.append(_copied_callbacks[0]())
        _copied_callbacks[0] = _callback_type(lambda: 3)


_copied_callbacks[0] = _callback_type(ReenteringCallback())
_copied_callbacks[0] = _callback_type(lambda: 2)
assert _finalized == [2]
assert _copied_callbacks[0]() == 3

# Pointer-valued scalar views retain their owner and write through to it.
for _type, _old, _new in (
    (ctypes.c_char_p, b"old", b"new"),
    (ctypes.c_wchar_p, "old", "new"),
):

    class IndexedStringPointer(_type):
        pass

    _owner = (IndexedStringPointer * 1)(_old)
    _view = _owner[0]
    _view.value = _new
    assert _view._b_base_ is _owner
    assert _owner[0].value == _new
    _copy = type(_owner)(_view)
    del _owner, _view
    gc.collect()
    assert _copy[0].value == _new


# Reentry must see matching pointer bytes and owners, including private copies.
class StringField(ctypes.Structure):
    _fields_ = [("text", ctypes.c_char_p)]


class ReenteringBytes(bytes):
    def __del__(self):
        if _replace_record:
            _strings[0] = StringField(_inner)
            _events.append(_strings[0].text)
        else:
            _strings.text = _inner
            _events.append(_strings.text)


_inner = b"inner replacement" * 20
for _replace_record in (False, True):
    _events = []
    if _replace_record:
        _strings = (StringField * 1)(StringField(ReenteringBytes(b"old")))
        _strings[0] = StringField(b"outer replacement")
        assert _strings[0].text == _inner
    else:
        _strings = StringField(ReenteringBytes(b"old"))
        _strings.text = b"outer replacement"
        assert _strings.text == _inner
    assert _events == [_inner]


# py_object views locate the referent in their owner, including after a copy.
class IndexedObject(ctypes.py_object):
    pass


_object = {}
_object_scalar = IndexedObject(_object)
_objects = (IndexedObject * 1)(_object_scalar)
assert type(_objects).from_buffer(memoryview(_objects))[0].value is _object
_object_pointer = ctypes.cast(_objects, ctypes.POINTER(IndexedObject))
_object_pointer[0] = _object_scalar
_object_scalar.value = object()
assert _object_pointer[0].value is _object
_copy = type(_objects)(_objects[0])
del _objects, _object_pointer
gc.collect()
assert _copy[0].value is _object
_copy[0].value = None
assert _copy[0].value is None


class RetentionKey(str):
    __hash__ = str.__hash__

    def __eq__(self, other):
        raise AssertionError("py_object lookup called key equality")


_copy[0].value = _object
_copy._objects.clear()
_copy._objects[RetentionKey("0:0")] = _object
assert _copy[0].value is _object

# Generated endian types return scalars, but their user subclasses return views.
_swapped = (
    ctypes.c_uint32.__ctype_be__
    if _sys.byteorder == "little"
    else ctypes.c_uint32.__ctype_le__
)
_words = (_swapped * 1).from_buffer_copy(b"\x01\x02\x03\x04")
_word_pointer = ctypes.cast(_words, ctypes.POINTER(_swapped))
_expected = int.from_bytes(
    bytes(_words), "big" if _sys.byteorder == "little" else "little"
)
assert type(_words[0]) is type(_word_pointer[0]) is int
assert _words[0] == _word_pointer[0] == _expected
_words[0] = 0x11223344
assert bytes(_words) == bytes(_swapped(0x11223344))
_word_pointer[0] = 0x55667788
assert bytes(_words) == bytes(_swapped(0x55667788))


class IndexedSwapped(_swapped):
    pass


_words = (IndexedSwapped * 1).from_buffer_copy(b"\x01\x02\x03\x04")
assert type(_words[0]) is IndexedSwapped
assert _words[0].value == int.from_bytes(bytes(_words), _sys.byteorder)

# Field assignment copies encoded instances; only numeric inputs are converted.
for _base in (ctypes.Structure, ctypes.Union):
    for _field_type in (_swapped, IndexedSwapped):

        class EndianField(_base):
            _fields_ = [("word", _field_type)]

        _value = _field_type(0x01020304)
        _aggregate = EndianField(_value)
        assert bytes(_aggregate) == bytes(_value)
        _field = EndianField.from_buffer_copy(bytes(_value)).word
        assert getattr(_field, "value", _field) == _value.value
        if _field_type is IndexedSwapped:
            _aggregate.word = 0x55667788
            assert bytes(_aggregate) == bytes(IndexedSwapped(0x55667788))

# Endian fields reject the native sibling rather than converting its bytes.
assert not issubclass(_swapped, ctypes.c_uint32)
for _base in (
    (ctypes.BigEndianStructure, ctypes.BigEndianUnion)
    if _sys.byteorder == "little"
    else (ctypes.LittleEndianStructure, ctypes.LittleEndianUnion)
):

    class ConvertedEndianField(_base):
        _fields_ = [("word", ctypes.c_uint32)]

    assert ConvertedEndianField._fields_[0][1] is _swapped
    try:
        ConvertedEndianField(ctypes.c_uint32(0x01020304))
    except TypeError:
        pass
    else:
        raise AssertionError("incompatible native field value accepted")
    _value = _swapped(0x01020304)
    assert bytes(ConvertedEndianField(_value)) == bytes(_value)


# Array assignment cannot complete a pointer field's missing target type.
class IncompletePointer(ctypes._Pointer):
    pass


class IncompletePointerField(ctypes.Structure):
    _fields_ = [("value", IncompletePointer)]


_field = IncompletePointerField()
_before = bytes(_field)
try:
    _field.value = (ctypes.c_int * 1)(42)
except TypeError:
    pass
else:
    raise AssertionError("array accepted for an incomplete pointer field")
assert bytes(_field) == _before

# Indexed writes use the element's conversion, not the input object's layout.
for _container, _input, _expected in (
    (ctypes.pointer(ctypes.c_float()), 1.5, 1.5),
    ((ctypes.c_char * 1)(), ctypes.c_char(b"x"), b"x"),
    ((ctypes.c_byte * 1)(), 257, 1),
):
    _container[0] = _input
    assert _container[0] == _expected

for _container in ((ctypes.c_int * 1)(), ctypes.pointer(ctypes.c_int())):
    try:
        _container[0] = ctypes.c_float(1.5)
    except TypeError:
        pass
    else:
        raise AssertionError("incompatible indexed value accepted")
    assert _container[0] == 0


class FailingFloat:
    def __float__(self):
        raise LookupError("conversion failed")


for _base in (ctypes.Structure, ctypes.Union):

    class ObjectAndFloat(_base):
        _fields_ = [("object", ctypes.py_object), ("number", ctypes.c_float)]

    _record = ObjectAndFloat()
    _value = object()
    _record.object = _value
    assert _record.object is _value
    _before = bytes(_record)
    try:
        _record.number = FailingFloat()
    except LookupError:
        pass
    else:
        raise AssertionError("conversion error suppressed")
    assert bytes(_record) == _before


# Tuple temporaries are finalized after their bytes have reached the target.
_events = []


class TemporaryRecord(ctypes.Structure):
    _fields_ = [("number", ctypes.c_int)]

    def __del__(self):
        if self._b_base_ is None:
            _events.append(_records[0].number)
            _records[0].number = 99


_records = (TemporaryRecord * 1)()
_records[0] = (42,)
assert _events == [42]
assert _records[0].number == 99


# Raw addresses can point inside the buffer already owned by a scalar view.
_events = []


class OwnedBytes(bytes):
    def __del__(self):
        _events.append("released")


class StringPointer(ctypes.c_char_p):
    pass


class OwnedStringField(ctypes.Structure):
    _fields_ = [("text", StringPointer)]


_record = OwnedStringField()
_record.text.value = OwnedBytes(b"owner")
_address = ctypes.cast(_record.text, ctypes.c_void_p).value
_record.text.value = _address + 1
gc.collect()
assert _events == []
assert _record.text.value == b"wner"

# Self-copy must not build a new private holder around every previous copy.
_marker = b"ctypes-self-copy-backing-store"
_strings = (StringField * 1)(StringField(_marker))
for _ in range(32):
    _strings[0] = _strings[0]
gc.collect()
assert (
    sum(
        type(obj) is tuple
        and any(type(item) is bytes and item == _marker + b"\0" for item in obj)
        for obj in gc.get_objects()
    )
    <= 1
)
assert _strings[0].text == _marker

# Mutating _type_ must not change the initialized setter or its write width.
for _code in ("h", "q", "f"):

    class MutableInt(ctypes.c_int):
        pass

    _array = (MutableInt * 2)(0, 0x1234)
    MutableInt._type_ = _code
    _array[0] = -2
    assert bytes(_array) == bytes(ctypes.c_int(-2)) + bytes(ctypes.c_int(0x1234))
    assert _array[0].value == -2
    assert MutableInt(-3).value == -3

for _scalar, _text_value in ((ctypes.c_char, b"ab"), (ctypes.c_wchar, "ab")):

    class MutableChar(_scalar):
        pass

    class TextField(ctypes.Structure):
        _fields_ = [("text", MutableChar * 4)]

    _text = TextField(_text_value)
    MutableChar._type_ = "i"
    assert _text.text == _text_value
    _text.text = _text_value
    assert _text.text == _text_value

if _sys.platform == "win32":

    class BSTR(ctypes._SimpleCData):
        _type_ = "X"

    _bstr = (BSTR * 1)()
    _bstr[0] = None
    assert bytes(_bstr) == bytes(ctypes.sizeof(ctypes.c_void_p))


# The py_object payload, not only the instance dictionary, closes this cycle.
class ObjectOwner:
    pass


_owner = ObjectOwner()
_owner.value = ctypes.py_object(_owner)
_ref = weakref.ref(_owner)
del _owner
gc.collect()
assert _ref() is None


# Values written through a pointer remain visible through sibling views.
for _borrowed in (False, True):
    _storage = bytearray(ctypes.sizeof(ctypes.py_object))
    _value = ctypes.py_object.from_buffer(_storage) if _borrowed else ctypes.py_object()
    _value.value = None
    _pointer = ctypes.cast(ctypes.byref(_value), ctypes.POINTER(ctypes.py_object))
    _sibling = ctypes.pointer(_value)
    _object = object()
    _pointer[0] = _object
    assert _value.value is _object and _sibling[0] is _object
    _pointer.contents.value = object()
    assert _value.value is _pointer[0] is _sibling[0]

if _sys.implementation.name == "rustpython":
    _cast_value = ctypes.py_object(None)
    _cast_ref = weakref.ref(_cast_value)
    _cast_pointer = ctypes.cast(
        ctypes.byref(_cast_value), ctypes.POINTER(ctypes.py_object)
    )
    del _cast_value
    gc.collect()
    assert _cast_ref() is not None
    del _cast_pointer
    gc.collect()
    assert _cast_ref() is None


# A temporary pointer field writes ownership into its containing structure.
class ObjectPointerHolder(ctypes.Structure):
    _fields_ = [("pointer", ctypes.POINTER(ctypes.py_object))]


_temporary_array = (ctypes.py_object * 1)(None)
_temporary_holder = ObjectPointerHolder(
    ctypes.cast(_temporary_array, ctypes.POINTER(ctypes.py_object))
)
_temporary_value = ObjectOwner()
_temporary_ref = weakref.ref(_temporary_value)
_temporary_holder.pointer[0] = _temporary_value
assert _temporary_array[0] is _temporary_value
# A weak writer registration must not extend the holder or value's lifetime.
del _temporary_value, _temporary_holder
gc.collect()
assert _temporary_ref() is None

# Pointer writes through a borrowed view must remain visible to its exporter.
_exporter_array = (ctypes.py_object * 1)(None)
_exporter_pointer = ctypes.pointer(ctypes.py_object.from_buffer(_exporter_array))
for _exporter_value in (object(), object()):
    _exporter_pointer[0] = _exporter_value
    assert _exporter_array[0] is _exporter_value
del _exporter_pointer, _exporter_array, _exporter_value

# A warmed writer must follow contents changes without retaining old storage.
_value = ctypes.py_object(None)
_pointer = ctypes.pointer(_value)
_pointer[0] = object()
_ref = weakref.ref(_value)
_value = ctypes.py_object(None)
_pointer.contents = _value
assert _ref() is None
_pointer[0] = _object
assert _value.value is _object

# Different owners can describe the same address.
_value = ctypes.py_object.from_buffer(_storage)
_pointer = ctypes.pointer(_value)
_pointer[0] = _object
_other = ctypes.py_object.from_buffer(_storage)
_pointer.contents = _other
_pointer[0] = object()
assert _other.value is _pointer[0]

_pointer = ctypes.cast(ctypes.byref(_value), ctypes.POINTER(ctypes.py_object))
_pointer.contents = _other
_pointer[0] = object()
assert _other.value is _pointer[0]
assert _value.value is _pointer[0]

# A cast array view also registers its enclosing allocation, not just contents.
_nested = ((ctypes.py_object * 1) * 1)()
_nested[0][0] = None
_pointer = ctypes.cast(_nested[0], ctypes.POINTER(ctypes.py_object))
_other = ctypes.py_object.from_buffer(_nested)
_pointer.contents = _other
_pointer[0] = object()
assert _nested[0][0] is _other.value is _pointer[0]

# Changing the index can bring a smaller cast source back into range.
_pair_type = ctypes.py_object * 2
_buffer = bytearray(ctypes.sizeof(_pair_type))
_small = (ctypes.py_object * 1).from_buffer(_buffer)
_large = _pair_type.from_buffer(_buffer)
_inner = ctypes.cast(ctypes.byref(_small), ctypes.POINTER(_pair_type))
_inner.contents = _large
_pointer = ctypes.cast(_inner.contents, ctypes.POINTER(ctypes.py_object))
_pointer[1] = object()
_pointer[0] = object()
assert _small[0] is _large[0] is _pointer[0]

_backing = bytearray(ctypes.sizeof(ctypes.py_object))
_value = ctypes.py_object.from_buffer(_backing)
_value.value = None
try:
    _backing.extend(b"x")
except BufferError:
    pass
else:
    raise AssertionError("assigning None released the buffer export")


# An aggregate assignment can overlap a reader smaller than the written value.
_small = (ctypes.py_object * 1).from_buffer(bytearray(ctypes.sizeof(_pair_type)))
_pointer = ctypes.cast(ctypes.byref(_small), ctypes.POINTER(_pair_type))
for _value in (object(), object()):
    _pointer[0] = _pair_type(_value, None)
    assert _small[0] is _value


# Independent exports share provenance without retaining the writer or value.
_storage = bytearray(ctypes.sizeof(ctypes.py_object))
_writer = ctypes.py_object.from_buffer(_storage)
_reader = ctypes.py_object.from_buffer(memoryview(_storage))
_pointer = ctypes.pointer(_writer)
for _direct in (False, True):
    _value = ObjectOwner()
    if _direct:
        _writer.value = _value
    else:
        _pointer[0] = _value
    assert _reader.value is _value
_ref = weakref.ref(_value)
del _value, _writer, _pointer
gc.collect()
assert _ref() is None

# Field caches on a cast pointer follow later assignments to the source array.
_roots = [ctypes.py_object(None), ctypes.py_object(None)]
_holders = (ObjectPointerHolder * 2)(
    *(ObjectPointerHolder(ctypes.pointer(_root)) for _root in _roots)
)
_pointer = ctypes.cast(_holders, ctypes.POINTER(ObjectPointerHolder))
for _index in range(2):
    _pointer[_index].pointer[0] = object()
_replacement = ctypes.py_object(None)
_holders[1].pointer = ctypes.pointer(_replacement)
_pointer[1].pointer[0] = _object
assert _replacement.value is _object

# A native exporter must see direct writes as well as pointer writes.
_native = (ctypes.py_object * 1)(None)
_view = ctypes.py_object.from_buffer(_native)
_view.value = _object
assert _native[0] is _object
