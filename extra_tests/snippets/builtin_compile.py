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
_check_flags_error(0x10000)
