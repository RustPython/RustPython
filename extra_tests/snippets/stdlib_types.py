import _ast
import platform
import types

from testutils import assert_raises

ns = types.SimpleNamespace(a=2, b="Rust")

assert ns.a == 2
assert ns.b == "Rust"
with assert_raises(AttributeError):
    _ = ns.c


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
    fn = _ast.FunctionDef("f", args, [], [], None, None)
    fn.lineno = 1
    fn.col_offset = 0
    fn.end_lineno = 1
    fn.end_col_offset = 0
    mod = _ast.Module([fn], [])
    mod.lineno = 1
    mod.col_offset = 0
    mod.end_lineno = 1
    mod.end_col_offset = 0
    compiled = compile(mod, "<stdlib_types_missing_type_params>", "exec")
    exec(compiled, {})


if platform.python_implementation() == "RustPython":
    _run_missing_type_params_regression()
