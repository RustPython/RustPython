"""Local _opcode conversion/order contract; shared binder wording is separate."""

import _opcode
import opcode
import struct

NOP = opcode.opmap["NOP"]
events = []


class Indexed:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        events.append("index")
        return self.value


class IndexErrorValue:
    def __index__(self):
        events.append("index-error")
        raise RuntimeError("ordinary index failure")


class IntOnly:
    def __int__(self):
        events.append("int")
        return NOP


class IntSubclass(int):
    def __index__(self):
        raise AssertionError("int subclass index override must not be called")


class BoolError:
    def __bool__(self):
        events.append("bool")
        raise AssertionError("jump must not be truth-tested")


def error(kind, call, message=None):
    try:
        call()
    except kind as exc:
        if message is not None:
            assert str(exc) == message, str(exc)
    else:
        raise AssertionError("expected " + kind.__name__)


assert _opcode.stack_effect(NOP) == 0
assert _opcode.stack_effect(NOP, None) == 0
assert _opcode.stack_effect(opcode.opmap["BUILD_LIST"], 3) == -2
error(
    ValueError,
    lambda: _opcode.stack_effect(opcode.opmap["BUILD_LIST"], -1),
    "invalid opcode or oparg",
)
assert _opcode.stack_effect(Indexed(NOP), Indexed(-1)) == 0
assert events == ["index", "index"]
assert _opcode.stack_effect(IntSubclass(NOP), IntSubclass(-1)) == 0
for jump in (None, False, True):
    assert _opcode.stack_effect(NOP, jump=jump) == 0
for jump in (0, 1, "", [], Indexed(1), BoolError()):
    events.clear()
    error(
        ValueError,
        lambda: _opcode.stack_effect(NOP, jump=jump),
        "stack_effect: jump must be False, True or None",
    )
    assert events == []

for function in (
    _opcode.is_valid,
    _opcode.has_arg,
    _opcode.has_const,
    _opcode.has_name,
    _opcode.has_jump,
    _opcode.has_free,
    _opcode.has_local,
    _opcode.has_exc,
):
    expected = function(NOP)
    events.clear()
    assert function(Indexed(NOP)) == expected
    assert events == ["index"]
    assert function(opcode=IntSubclass(NOP)) == expected
    for value in (-(1 << 31) - 1, 1 << 31, 1 << 100):
        error(
            OverflowError,
            lambda: function(value),
            "Python int too large to convert to C int",
        )
    events.clear()
    error(TypeError, lambda: function(Indexed(NOP), 0))
    assert events == []
    error(TypeError, lambda: function(IntOnly()))
    assert events == []

# Constant-effect NOP keeps extreme conversion values out of arithmetic formulas.
long_bits = struct.calcsize("@l") * 8
for value in (
    -(1 << 31),
    (1 << 31) - 1,
    -(1 << (long_bits - 1)),
    (1 << (long_bits - 1)) - 1,
):
    assert _opcode.stack_effect(NOP, value) == 0
for value in (-(1 << (long_bits - 1)) - 1, 1 << (long_bits - 1)):
    error(
        OverflowError,
        lambda: _opcode.stack_effect(NOP, value),
        "Python int too large to convert to C long",
    )
for value in (-(1 << 31) - 1, 1 << 31):
    error(
        OverflowError,
        lambda: _opcode.stack_effect(value),
        "Python int too large to convert to C int",
    )

events.clear()
error(
    RuntimeError,
    lambda: _opcode.stack_effect(IndexErrorValue(), IndexErrorValue()),
    "ordinary index failure",
)
assert events == ["index-error"]
events.clear()
error(
    OverflowError,
    lambda: _opcode.stack_effect(1 << 100, IndexErrorValue()),
    "Python int too large to convert to C int",
)
assert events == []
error(
    RuntimeError,
    lambda: _opcode.stack_effect(-1, IndexErrorValue(), jump=BoolError()),
    "ordinary index failure",
)
assert events == ["index-error"]
events.clear()
error(
    ValueError,
    lambda: _opcode.stack_effect(-1, 0, jump=BoolError()),
    "stack_effect: jump must be False, True or None",
)
assert events == []
for call in (
    lambda: _opcode.stack_effect(Indexed(NOP), 0, False),
    lambda: _opcode.stack_effect(Indexed(NOP), other=False),
    lambda: _opcode.stack_effect(Indexed(NOP), oparg=Indexed(0)),
):
    error(TypeError, call)
    assert events == []


class LifetimeOperand:
    def __index__(self):
        events.append("temporary-index")
        return 0

    def __del__(self):
        events.append("temporary-release")


# Acyclic temporaries are released on both success and validation error.
events.clear()
assert _opcode.stack_effect(NOP, LifetimeOperand()) == 0
assert events == ["temporary-index", "temporary-release"]
events.clear()
error(
    ValueError,
    lambda: _opcode.stack_effect(NOP, LifetimeOperand(), jump=0),
    "stack_effect: jump must be False, True or None",
)
assert events == ["temporary-index", "temporary-release"]
