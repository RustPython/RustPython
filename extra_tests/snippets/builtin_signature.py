import builtins
import inspect
import os

# __text_signature__ is generated from the Rust parameter list, so it must not
# describe parameters the function does not actually take, and must mark the
# ones it does take as positional-only.

# No phantom `module` parameter. The signature marks it as `$module` and
# `__self__` is the module, so inspect strips it.
for f in (len, abs, hash, id, repr, bin, ord, divmod, hex, oct, chr, callable):
    assert "module" not in inspect.signature(f).parameters, f.__name__

# Plain arguments bind through take_positional(), so they are positional-only.
try:
    len(obj=[1, 2])
except TypeError:
    pass
else:
    raise AssertionError("len() should not accept keyword arguments")

assert str(inspect.signature(len)) == "(obj, /)"
assert str(inspect.signature(abs)) == "(number, /)"
assert str(inspect.signature(hash)) == "(obj, /)"
assert str(inspect.signature(chr)) == "(i, /)"
assert str(inspect.signature(callable)) == "(obj, /)"

assert (
    inspect.signature(len).parameters["obj"].kind == inspect.Parameter.POSITIONAL_ONLY
)

# *args/**kwargs cannot be followed by `/`; collector names also match Clinic.
assert breakpoint.__text_signature__ == "($module, /, *args, **kws)"
assert str(inspect.signature(breakpoint)) == "(*args, **kws)"
# Collector names are descriptive, not reserved keyword parameters.
import sys

original_breakpointhook = sys.breakpointhook
try:
    sys.breakpointhook = lambda *args, **kws: (args, kws)
    assert breakpoint(1, args=2, kws=3, kwargs=4) == (
        (1,),
        {"args": 2, "kws": 3, "kwargs": 4},
    )
finally:
    sys.breakpointhook = original_breakpointhook
breakpoint_kinds = [p.kind for p in inspect.signature(breakpoint).parameters.values()]
assert breakpoint_kinds == [
    inspect.Parameter.VAR_POSITIONAL,
    inspect.Parameter.VAR_KEYWORD,
], breakpoint_kinds

# Parameter names follow CPython, so signatures are directly comparable.
assert str(inspect.signature(bin)) == "(integer, /)"
assert str(inspect.signature(ord)) == "(character, /)"
assert str(inspect.signature(divmod)) == "(x, y, /)"
assert str(inspect.signature(hasattr)) == "(obj, name, /)"
assert str(inspect.signature(setattr)) == "(obj, name, value, /)"
assert str(inspect.signature(delattr)) == "(obj, name, /)"
assert str(inspect.signature(isinstance)) == "(obj, class_or_tuple, /)"
assert str(inspect.signature(issubclass)) == "(cls, class_or_tuple, /)"
assert str(inspect.signature(aiter)) == "(async_iterable, /)"

# A generated signature is stored in __doc__ and read back out of it, so it
# needs the `--` terminator even when the function has no documentation.
assert str(inspect.signature(os.getpid)) == "()"
assert str(inspect.signature(os.getcwd)) == "()"

# Methods report a signature too, with the receiver marked so that binding a
# method drops it.
assert str(inspect.signature(list.__dir__)) == "(self, /)"
assert str(inspect.signature([].__dir__)) == "()"
assert str(inspect.signature(float.fromhex)) == "(string, /)"

# Functions whose arguments come from a FromArgs struct report the struct's
# parameters.
assert round.__text_signature__ == "($module, /, number, ndigits=None)"
assert sum.__text_signature__ == "($module, iterable, /, start=0)"
assert str(inspect.signature(round)) == "(number, ndigits=None)"
assert str(inspect.signature(sum)) == "(iterable, /, start=0)"

# A keyword-only argument between two positional ones used to put `/` after
# `*`, which inspect cannot parse. Arguments are ordered like CPython's, so
# path and mode take keywords. Windows renders follow_symlinks differently.
if hasattr(os, "chmod"):
    chmod_signature = os.chmod.__text_signature__
    assert chmod_signature.startswith("($module, /, path, mode, *, dir_fd=None"), (
        chmod_signature
    )

# Omitted arguments keep their runtime sentinels while displaying CPython 3.15's
# public defaults. In particular, an omitted compression dictionary is not None.
import binascii
import signal
import zlib

for f in (binascii.hexlify, binascii.b2a_hex):
    assert f.__text_signature__ == (
        "($module, /, data, sep=<unrepresentable>, bytes_per_sep=1)"
    )
    assert f(b"abc", b":") == f(b"abc", b":", 1) == b"61:62:63"

if hasattr(signal, "setitimer"):
    assert signal.setitimer.__text_signature__ == (
        "($module, which, seconds, interval=0.0, /)"
    )
    assert inspect.signature(signal.setitimer).parameters["interval"].default == 0.0

assert " ".join(zlib.compressobj.__text_signature__.split()) == (
    "($module, /, level=Z_DEFAULT_COMPRESSION, method=DEFLATED, "
    "wbits=MAX_WBITS, memLevel=DEF_MEM_LEVEL, "
    "strategy=Z_DEFAULT_STRATEGY, zdict=<unrepresentable>)"
)
try:
    zlib.compressobj(zdict=None)
except TypeError:
    pass
else:
    raise AssertionError("None is not an omitted compression dictionary")

assert zlib._ZlibDecompressor.__text_signature__ == "(wbits=MAX_WBITS, zdict=b'')"
assert (
    inspect.signature(zlib._ZlibDecompressor).parameters["wbits"].default
    == zlib.MAX_WBITS
)
assert zlib._ZlibDecompressor().decompress(zlib.compress(b"signature defaults")) == (
    b"signature defaults"
)

# Clinic keeps platform-dependent defaults symbolic rather than hard-coding
# the 64-bit value. inspect resolves the symbol for the running interpreter.
import sys

for sequence_type in (list, tuple):
    assert sequence_type.index.__text_signature__ == (
        "($self, value, start=0, stop=sys.maxsize, /)"
    )
    index_signature = inspect.signature(sequence_type.index)
    assert index_signature.parameters["stop"].default == sys.maxsize

assert str.maketrans.__text_signature__ == (
    "(x, y=<unrepresentable>, z=<unrepresentable>, /)"
)

if hasattr(os, "utime"):
    # ns is deliberately unrepresentable; only times has a None default.
    assert " ".join(os.utime.__text_signature__.split()) == (
        "($module, /, path, times=None, *, ns=<unrepresentable>, "
        "dir_fd=None, follow_symlinks=True)"
    )

# Buffered stream constructors retain Clinic's symbolic module default.
import _io

for stream_type in (_io.BufferedReader, _io.BufferedWriter, _io.BufferedRandom):
    assert stream_type.__text_signature__ == "(raw, buffer_size=DEFAULT_BUFFER_SIZE)"
    assert (
        inspect.signature(stream_type).parameters["buffer_size"].default
        == _io.DEFAULT_BUFFER_SIZE
    )

assert _io.BufferedRWPair.__text_signature__ == (
    "(reader, writer, buffer_size=DEFAULT_BUFFER_SIZE, /)"
)
assert (
    inspect.signature(_io.BufferedRWPair).parameters["buffer_size"].default
    == _io.DEFAULT_BUFFER_SIZE
)

# CPython's tp_new_wrapper hides its bound defining type, but still requires
# the caller to supply the subtype as the first positional argument. The raw
# signature intentionally has no slash, unlike other slot descriptors.
new_wrapper_types = (
    "object",
    "type",
    "bool",
    "bytearray",
    "bytes",
    "complex",
    "dict",
    "float",
    "frozenset",
    "int",
    "list",
    "memoryview",
    "range",
    "set",
    "slice",
    "str",
    "tuple",
    "BaseException",
    "Exception",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "EOFError",
    "ImportError",
    "IndexError",
    "KeyError",
    "LookupError",
    "MemoryError",
    "NameError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "ReferenceError",
    "RuntimeError",
    "StopIteration",
    "SyntaxError",
    "SystemError",
    "SystemExit",
    "TypeError",
    "ValueError",
    "ZeroDivisionError",
    "Warning",
    "UserWarning",
    "RuntimeWarning",
    "DeprecationWarning",
    "FutureWarning",
    "UnicodeError",
)
for type_name in new_wrapper_types:
    cls = getattr(builtins, type_name)
    new = cls.__new__
    assert new.__text_signature__ == "($type, *args, **kwargs)", type_name
    assert str(inspect.signature(new)) == "(*args, **kwargs)", type_name
    assert isinstance(new.__self__, type), type_name


class NewList(list):
    pass


assert NewList.__new__ is list.__new__
assert NewList.__new__.__self__ is list
assert dict.__new__(dict, answer=42) == {}
assert int.__new__(int, "10", base=2) == 2
# The hidden $type in the raw signature does not make the explicit subtype
# keyword-bindable. Do not silently alter dispatch when matching its spelling.
for new, kwargs in ((object.__new__, {"type": object}), (list.__new__, {"cls": list})):
    try:
        new(**kwargs)
    except TypeError:
        pass
    else:
        raise AssertionError("the subtype must remain positional")
# CPython falls back to METH_NOARGS/METH_O signatures for these non-Clinic
# functions. The hidden marker is $self even though they bind a module.
import importlib

flags_signatures = {
    "_ast": {
        "_is_abstract": "($self, object, /)",
    },
    "_codecs_cn": {
        "getcodec": "($self, object, /)",
    },
    "_codecs_hk": {
        "getcodec": "($self, object, /)",
    },
    "_codecs_iso2022": {
        "getcodec": "($self, object, /)",
    },
    "_codecs_jp": {
        "getcodec": "($self, object, /)",
    },
    "_codecs_kr": {
        "getcodec": "($self, object, /)",
    },
    "_codecs_tw": {
        "getcodec": "($self, object, /)",
    },
    "_ctypes": {
        "Py_DECREF": "($self, object, /)",
        "Py_INCREF": "($self, object, /)",
        "alignment": "($self, object, /)",
        "buffer_info": "($self, object, /)",
        "dllist": "($self, /)",
        "get_errno": "($self, /)",
    },
    "_interpchannels": {
        "list_all": "($self, /)",
    },
    "_socket": {
        "close": "($self, object, /)",
        "dup": "($self, object, /)",
        "getdefaulttimeout": "($self, /)",
        "gethostname": "($self, /)",
        "if_nameindex": "($self, /)",
        "setdefaulttimeout": "($self, object, /)",
    },
    "_stat": {
        "S_IFMT": "($self, object, /)",
        "S_IMODE": "($self, object, /)",
        "S_ISBLK": "($self, object, /)",
        "S_ISCHR": "($self, object, /)",
        "S_ISDIR": "($self, object, /)",
        "S_ISDOOR": "($self, object, /)",
        "S_ISFIFO": "($self, object, /)",
        "S_ISLNK": "($self, object, /)",
        "S_ISPORT": "($self, object, /)",
        "S_ISREG": "($self, object, /)",
        "S_ISSOCK": "($self, object, /)",
        "S_ISWHT": "($self, object, /)",
        "filemode": "($self, object, /)",
    },
    "_string": {
        "formatter_field_name_split": "($self, object, /)",
        "formatter_parser": "($self, object, /)",
    },
    "_uuid": {
        "generate_time_safe": "($self, /)",
    },
    "time": {
        "mktime": "($self, object, /)",
        "monotonic": "($self, /)",
        "monotonic_ns": "($self, /)",
        "perf_counter": "($self, /)",
        "perf_counter_ns": "($self, /)",
        "process_time": "($self, /)",
        "process_time_ns": "($self, /)",
        "sleep": "($self, object, /)",
        "thread_time": "($self, /)",
        "thread_time_ns": "($self, /)",
        "time": "($self, /)",
        "time_ns": "($self, /)",
    },
}
for module_name, signatures in flags_signatures.items():
    try:
        module = importlib.import_module(module_name)
    except ImportError:
        # Some native modules are unavailable on WASI or disabled by features.
        continue
    for function_name, expected in signatures.items():
        function = getattr(module, function_name, None)
        if function is None:
            # Unix-only symbols are not present on every platform.
            continue
        assert function.__text_signature__ == expected, (
            module_name,
            function_name,
            function.__text_signature__,
        )
        assert function.__self__ is module, (module_name, function_name)
        displayed = "(object, /)" if "object" in expected else "()"
        assert str(inspect.signature(function)) == displayed

# Ordinary Clinic functions keep their module marker and real parameter names.
assert len.__text_signature__ == "($module, obj, /)"
