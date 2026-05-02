from testutils import assert_raises


# CPython parity: encoding declarations (`# -*- coding: ... -*-`) only apply
# to bytes input. For `str` source the source is already decoded and the
# declaration must be ignored, including the "unknown encoding" error path.
compile("# -*- coding: badencoding -*-\nx = 1\n", "tmp", "exec")
compile("# -*- coding: latin1 -*-\nx = 1\n", "tmp", "exec")
compile("# -*- coding: utf-8 -*-\nx = 1\n", "tmp", "exec")

# Bytes input keeps applying the declaration, so a bogus encoding still
# raises SyntaxError.
assert_raises(
    SyntaxError, compile, b"# -*- coding: badencoding -*-\nx = 1\n", "tmp", "exec"
)


# CPython mode error wording. Both the missing `compile()` prefix and the
# Oxford-comma / quote style come from the parser's generic error message;
# `compile()` overrides it to match CPython exactly.
def _check_mode_error(mode_str):
    try:
        compile("x = 1", "<test>", mode_str)
    except ValueError as e:
        assert str(e) == "compile() mode must be 'exec', 'eval' or 'single'", repr(e)
    else:
        raise AssertionError(f"expected ValueError for mode={mode_str!r}")


for bad in ("bogus", "", "BAD", "__bogus__"):
    _check_mode_error(bad)


# `func_type` is only valid when PyCF_ONLY_AST (= 1024) is set. Plain text
# source without the flag must raise the specific "requires flag" error,
# not the generic "invalid mode" one.
try:
    compile("def f(x): pass", "<test>", "func_type")
except ValueError as e:
    assert (
        str(e) == "compile() mode 'func_type' requires flag PyCF_ONLY_AST"
    ), repr(e)
else:
    raise AssertionError("expected ValueError for func_type without PyCF_ONLY_AST")
