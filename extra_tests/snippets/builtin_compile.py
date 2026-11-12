import __future__

import ast

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
assert barry_code.co_flags & barry_flag

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


def _expect_incomplete(source, mode):
    try:
        compile(source, "<test>", mode, flags=PY_CF_ALLOW_INCOMPLETE_INPUT)
    except _IncompleteInputError:
        return
    raise AssertionError(f"expected _IncompleteInputError for {source!r} {mode}")


def _expect_syntax(source, mode):
    try:
        compile(source, "<test>", mode, flags=PY_CF_ALLOW_INCOMPLETE_INPUT)
    except _IncompleteInputError:
        raise AssertionError(f"unexpected _IncompleteInputError for {source!r} {mode}")
    except SyntaxError:
        return
    raise AssertionError(f"expected SyntaxError for {source!r} {mode}")


# eval/single treat empty input as incomplete; exec accepts an empty module.
_expect_incomplete("", "eval")
_expect_incomplete("", "single")
_expect_incomplete("\n", "eval")
_expect_incomplete("\n", "single")
compile("", "<test>", "exec", flags=PY_CF_ALLOW_INCOMPLETE_INPUT)

# A plain string still open at EOF is incomplete, except in exec, which
# appends a newline and so sees an unescaped newline instead.
_expect_incomplete("'abc", "eval")
_expect_incomplete("'abc", "single")
_expect_syntax("'abc", "exec")
# A final `\` eats exec's implicit newline, so the string is still open at EOF.
_expect_incomplete("a = 'a\\", "exec")
_expect_incomplete("a = 'a\\", "single")
_expect_syntax("a = 'a\\", "eval")
_expect_incomplete("'''abc", "eval")
_expect_incomplete("'''abc", "exec")

# Single-quoted f/t-strings never set E_EOLS.
_expect_syntax("f'", "eval")
_expect_syntax("f'", "exec")
_expect_syntax("f'", "single")
_expect_syntax("f'{1+'", "eval")
# An unclosed `{` in the field is EOF (`E_EOF`), so it is incomplete.
_expect_incomplete("f'{", "eval")
_expect_incomplete("f'{1", "eval")
_expect_incomplete("f'''", "eval")
_expect_incomplete("f'''", "exec")

# exec turns a final `\` into a continuation then EOF; single/eval do not.
_expect_syntax("9+ \\", "eval")
_expect_syntax("9+ \\", "single")
_expect_incomplete("9+ \\", "exec")

# Non-tokenizer whitespace is a hard error, not blank input.
_expect_syntax("\xa0", "eval")
_expect_syntax("\xa0", "exec")
_expect_syntax("\x0b", "eval")

# A lambda default `=` is not an assignment statement.
_expect_incomplete("lambda x='abc", "eval")
# A Unicode identifier plus a quote is a plain string, not an f-string.
_expect_incomplete("éf'", "eval")
# Comment-only single input is invalid syntax, not an indent error.
try:
    compile("# a", "<test>", "single")
except IndentationError:
    raise AssertionError("comment-only single input should not be IndentationError")
except SyntaxError:
    pass
else:
    raise AssertionError("expected SyntaxError for comment-only single input")
_expect_incomplete("# a", "single")
_expect_incomplete(" #x", "single")

# The source is encoded before it is parsed, so a lone surrogate has to be
# reported rather than assumed away.
with assert_raises(UnicodeEncodeError):
    compile(chr(0xD800), "<test>", "eval")

# Initial indentation in eval input precedes later tokenizer errors, with the
# same locations whether compile returns bytecode or an AST.
for source, line, column in (
    (" 1", 1, 1),
    ("\n -1", 2, 1),
    ("\n\t-1", 2, 1),
    ("\n\f -1", 2, 2),
    ("\n  # comment\n  -1", 3, 2),
    ("\n    \\\n  -1", 3, 2),
    ("\n    \\\n    (\\\n\\ ", 3, 4),
):
    for flags in (0, ast.PyCF_ONLY_AST):
        try:
            compile(source, "<test>", "eval", flags=flags)
        except IndentationError as exc:
            assert exc.msg == "unexpected indent"
            assert (exc.lineno, exc.offset, exc.end_lineno, exc.end_offset) == (
                line,
                column,
                line,
                -1,
            )
        else:
            raise AssertionError("initial eval indentation was accepted")

assert eval(compile("(\n  1\n)", "<test>", "eval")) == 1

# Physical CR and CRLF are translated before lexing, even inside literals.
# Keep explicit backslash-r escapes distinct from physical line endings.
for ending in ("\r", "\r\n", "\n\r", "\r\r\n"):
    normalized = ending.replace("\r\n", "\n").replace("\r", "\n")
    for prefix in ("", "r", "b", "f"):
        source = prefix + '"""a' + ending + 'b"""'
        expected = "a" + normalized + "b"
        if prefix == "b":
            expected = expected.encode()
        assert eval(source) == expected
        assert eval(source.encode()) == expected
        assert eval(compile(source, "<newlines>", "eval")) == expected
        tree = ast.parse(source, mode="eval")
        assert eval(compile(tree, "<newlines>", "eval")) == expected
        namespace = {}
        exec("value = " + source, namespace)
        assert namespace["value"] == expected
    assert eval('t"""a' + ending + 'b"""').strings == ("a" + normalized + "b",)
    namespace = {}
    exec('def f():\r    """a' + ending + '    b"""\r    pass', namespace)
    assert namespace["f"].__doc__ == "a" + normalized + "b"
    assert eval('"a\\' + ending[:1] + 'b"') == "ab"
assert eval(r'"a\rb"') == "a\rb"
assert eval(r'r"a\rb"') == r"a\rb"

# Warnings and SyntaxError text must use the same source as the parser.
import warnings

for source in ('"""a\r\\q"""', '"""a\r\\q""" + )', "1\r+", '"a\rb"'):
    results = []
    for text in (source, source.replace("\r", "\n")):
        with warnings.catch_warnings(record=True) as captured:
            warnings.simplefilter("always")
            try:
                eval(text)
            except SyntaxError as exc:
                result = (exc.msg, exc.lineno, exc.offset, exc.text)
            else:
                result = None
            results.append((result, [(str(w.message), w.lineno) for w in captured]))
    assert results[0] == results[1], (source, results)

# Audit hooks see the caller's bytes, not the parser's translated buffer.
import sys

newline_audit_events = []
newline_audit_source = '"""audit\rsource"""'


def record_newline_compile(event, args):
    if event == "compile" and args[0] == newline_audit_source.encode():
        newline_audit_events.append(args[0])


sys.addaudithook(record_newline_compile)
assert eval(newline_audit_source) == "audit\nsource"
assert newline_audit_events == [newline_audit_source.encode()]
