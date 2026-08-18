import inspect
import sys

# __text_signature__ is generated from the Rust parameter list, so it must not
# describe parameters the function does not actually take, and must mark the
# ones it does take as positional-only.

# No phantom `module` parameter. RustPython's #[pyfunction]s take no module
# argument, so `__self__` is None and inspect has nothing to strip.
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

if sys.implementation.name == "rustpython":
    # Functions whose Rust arguments are destructuring patterns rather than
    # plain names get no signature at all, instead of emitting text that is not
    # valid Python and makes inspect.signature() raise "invalid signature".
    #
    # CPython does have signatures for these, hand-written via Argument Clinic.
    # We cannot derive them until FromArgs reports the parameters of its own
    # structs, so until then we report no signature, which is at least how
    # CPython behaves for the builtins it has no signature for.
    for f in (round, sum):
        assert f.__text_signature__ is None, f.__name__
        try:
            inspect.signature(f)
        except ValueError as e:
            assert "no signature found" in str(e), str(e)
        else:
            raise AssertionError(f"{f.__name__} should have no signature")
