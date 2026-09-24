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
assert str(inspect.signature(abs)) == "(x, /)"
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
assert str(inspect.signature(bin)) == "(number, /)"
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
