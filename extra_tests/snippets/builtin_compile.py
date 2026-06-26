import __future__

import ast
import sys

from testutils import assert_raises

# compile() basic mode acceptance
assert isinstance(
    compile("x = 1", "<test>", "exec"), type(compile("", "<test>", "exec"))
)
assert compile("1 + 1", "<test>", "eval") is not None
assert compile("1", "<test>", "single") is not None

# `optimize` accepts -1 (use config default), 0, 1, 2 only.
# Anything else raises ValueError with CPython's exact wording.
for ok in (-1, 0, 1, 2):
    compile("x = 1", "<test>", "exec", optimize=ok)


def _check_optimize_error(value):
    try:
        compile("x = 1", "<test>", "exec", optimize=value)
    except ValueError as e:
        assert str(e) == "compile(): invalid optimize value", repr(e)
    else:
        raise AssertionError(f"expected ValueError for optimize={value!r}")


for bad in (3, 4, 99, 255, 256, 1000, -2, -99, -128):
    _check_optimize_error(bad)

# Huge `optimize` values raise OverflowError during argument conversion,
# not ValueError. The exact wording differs from CPython here (Rust i32
# vs C int) — checking the type only, matching test_compile.py.
assert_raises(OverflowError, compile, "x = 1", "<test>", "exec", optimize=1 << 1000)


# Unrecognised `flags` bits raise ValueError. CPython uses British spelling
# ("unrecognised") so the message must match exactly.
def _check_flags_error(flags):
    try:
        compile("x = 1", "<test>", "exec", flags=flags)
    except ValueError as e:
        assert str(e) == "compile(): unrecognised flags", repr(e)
    else:
        raise AssertionError(f"expected ValueError for flags={flags!r}")


_check_flags_error(99999)
_check_flags_error(0x100)
_check_flags_error(0x800)
_check_flags_error(0x10000)


ns = {}
exec(
    "from __future__ import annotations\n"
    "inherited = compile('x: __debug__\\n', '<test>', 'exec')\n"
    "not_inherited = compile('x: __debug__\\n', '<test>', 'exec', dont_inherit=True)\n",
    ns,
)
assert ns["inherited"].co_flags & 0x1000000
assert not (ns["not_inherited"].co_flags & 0x1000000)

barry_flag = __future__.barry_as_FLUFL.compiler_flag
barry_code = compile("x = 1", "<test>", "exec", flags=barry_flag)
compile("from __future__ import barry_as_FLUFL\nx = 1\n", "<test>", "exec")
if sys.implementation.name == "rustpython":
    assert not (barry_code.co_flags & barry_flag)

n = ast.parse('x = "# type: int"\n', type_comments=True)
assert n.body[0].type_comment is None
n = ast.parse("x = '# type: int'\n", type_comments=True)
assert n.body[0].type_comment is None
n = ast.parse('x = "abc" # type: str\n', type_comments=True)
assert n.body[0].type_comment == "str"
n = ast.parse("x = 1 # type: ignore[excuse]\n", type_comments=True)
assert [(ti.lineno, ti.tag) for ti in n.type_ignores] == [(1, "[excuse]")]


compile("() -> int", "<test>", "func_type", flags=ast.PyCF_ONLY_AST)
func_type_tree = compile(
    '("a,b", str) -> int', "<test>", "func_type", flags=ast.PyCF_ONLY_AST
)
assert len(func_type_tree.argtypes) == 2
assert func_type_tree.argtypes[0].value == "a,b"
func_type_tree = compile(
    "(int, *str, **Any) -> float",
    "<test>",
    "func_type",
    flags=ast.PyCF_ONLY_AST,
)
assert [arg.id for arg in func_type_tree.argtypes] == ["int", "str", "Any"]
assert_raises(
    SyntaxError,
    compile,
    "int -> str",
    "<test>",
    "func_type",
    flags=ast.PyCF_ONLY_AST,
)
assert_raises(
    SyntaxError,
    compile,
    "(x=1) -> str",
    "<test>",
    "func_type",
    flags=ast.PyCF_ONLY_AST,
)
assert_raises(
    SyntaxError,
    compile,
    "(int,) -> str",
    "<test>",
    "func_type",
    flags=ast.PyCF_ONLY_AST,
)
PY_CF_DONT_IMPLY_DEDENT = 0x0200
PY_CF_ALLOW_INCOMPLETE_INPUT = 0x4000
compile(b"# coding: latin-1\nx = '\xe9'\n", "<test>", "exec")
compile("if 1:\n pass", "<test>", "single")
assert_raises(
    SyntaxError,
    compile,
    "if 1:\n pass",
    "<test>",
    "single",
    flags=PY_CF_DONT_IMPLY_DEDENT,
)
compile(
    "if 1:\n pass\n",
    "<test>",
    "single",
    flags=PY_CF_DONT_IMPLY_DEDENT | PY_CF_ALLOW_INCOMPLETE_INPUT,
)
try:
    compile(
        "if 1:\n pass",
        "<test>",
        "single",
        flags=PY_CF_DONT_IMPLY_DEDENT | PY_CF_ALLOW_INCOMPLETE_INPUT,
    )
except _IncompleteInputError as exc:
    assert exc.args[0] == "incomplete input", repr(exc)
else:
    raise AssertionError("expected _IncompleteInputError")
