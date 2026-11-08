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

# *args/**kwargs cannot be followed by `/`. The parameter names themselves still
# differ from CPython here, which is out of scope.
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
