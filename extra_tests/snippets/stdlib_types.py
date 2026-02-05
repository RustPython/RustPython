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
