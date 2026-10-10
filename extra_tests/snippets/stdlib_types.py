import _ast
import platform
import sys
import types

from testutils import assert_raises

ns = types.SimpleNamespace(a=2, b="Rust")

assert ns.a == 2
assert ns.b == "Rust"
with assert_raises(AttributeError):
    _ = ns.c

import collections.abc

assert types.SimpleNamespace.__hash__ is None
assert not isinstance(ns, collections.abc.Hashable)
with assert_raises(TypeError):
    hash(ns)


def _run_missing_type_params_regression():
    args = _ast.arguments(
        posonlyargs=[],
        args=[],
        vararg=None,
        kwonlyargs=[],
        kw_defaults=[],
        kwarg=None,
        defaults=[],
    )
    pass_stmt = _ast.Pass(lineno=1, col_offset=4, end_lineno=1, end_col_offset=8)
    fn = _ast.FunctionDef("f", args, [pass_stmt], [], None, None)
    fn.lineno = 1
    fn.col_offset = 0
    fn.end_lineno = 1
    fn.end_col_offset = 8
    mod = _ast.Module([fn], [])
    compiled = compile(mod, "<stdlib_types_missing_type_params>", "exec")
    exec(compiled, {})


_run_missing_type_params_regression()

if sys.implementation.name == "rustpython":
    # __parameters__ is computed when the alias is built, and the walk descends
    # into every list and tuple argument, so a self-referential or deeply
    # nested argument must be caught. CPython, which also runs this snippet,
    # does not walk into plain lists at all.
    self_referential = []
    self_referential.append(self_referential)
    with assert_raises(RecursionError):
        list[self_referential]

    nested = [0]
    for _ in range(100_000):
        nested = [nested]
    with assert_raises(RecursionError):
        list[nested]

    # hashing an alias walks the same shape. Release inlining of the hash
    # slot needs more depth than debug to trip the native stack guard.
    deep_alias = int
    for _ in range(500_000):
        deep_alias = list[deep_alias]
    with assert_raises(RecursionError):
        hash(deep_alias)


# FunctionType preserves the code qualified name when a name is supplied.
def outer():
    def nested():
        pass

    return nested


original = outer()
for name in ("renamed", ""):
    function = types.FunctionType(original.__code__, globals(), name)
    assert function.__name__ == name
    assert function.__qualname__ == "outer.<locals>.nested"
