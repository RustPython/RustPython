"""Stable canonical code exports, including the first read during monitoring."""

import _frozen_importlib
import marshal
import sys
import types

SOURCE = """def total(values):
    result = 0
    for value in values:
        result += value
    return result
"""
FILENAME = "canonical_code_storage.py"


def make_function():
    namespace = {}
    exec(compile(SOURCE, FILENAME, "exec"), namespace)
    return namespace["total"]


reference = make_function()
expected = reference.__code__.co_code
assert reference.__code__.co_code is expected
assert reference.__code__.co_code == expected
for _ in range(100):
    assert reference((1, 2, 3)) == 6
assert reference.__code__.co_code is expected

original = reference.__code__
replaced = original.replace()
decoded = marshal.loads(marshal.dumps(original, 6))
constructed = types.CodeType(
    original.co_argcount,
    original.co_posonlyargcount,
    original.co_kwonlyargcount,
    original.co_nlocals,
    original.co_stacksize,
    original.co_flags,
    expected,
    original.co_consts,
    original.co_names,
    original.co_varnames,
    FILENAME,
    original.co_name,
    original.co_qualname,
    original.co_firstlineno,
    original.co_linetable,
    original.co_exceptiontable,
    original.co_freevars,
    original.co_cellvars,
)
for code in (replaced, decoded, constructed):
    first = code.co_code
    assert first == expected
    assert code.co_code is first
    assert types.FunctionType(code, {})([2, 3, 4]) == 9
    assert code.co_code is first

# Complete byte spellings from the pinned final 3.15 oracle for this ordinary
# yield-from fixture. Changing flags must normalize nonzero GET_ITER again.
delegate = compile(
    "def delegate(values):\n    yield from values\n", FILENAME, "exec"
).co_consts[0]
ordinary_delegate = bytes(
    [
        32,
        0,
        29,
        0,
        128,
        0,
        0,
        0,
        83,
        0,
        70,
        3,
        0,
        0,
        80,
        7,
        105,
        4,
        0,
        0,
        119,
        1,
        128,
        2,
        0,
        0,
        75,
        6,
        10,
        0,
        29,
        0,
        80,
        7,
        33,
        0,
        7,
        0,
        75,
        6,
        51,
        3,
        104,
        1,
    ]
)
coroutine_delegate = bytes(
    [
        32,
        0,
        29,
        0,
        128,
        0,
        0,
        0,
        83,
        0,
        70,
        2,
        0,
        0,
        80,
        7,
        105,
        4,
        0,
        0,
        119,
        1,
        128,
        2,
        0,
        0,
        75,
        6,
        10,
        0,
        29,
        0,
        80,
        7,
        33,
        0,
        7,
        0,
        75,
        6,
        51,
        3,
        104,
        1,
    ]
)
assert delegate.co_code == ordinary_delegate
changed = delegate.replace(co_flags=delegate.co_flags | 0x100)
assert changed.co_code == coroutine_delegate
assert changed.co_code is changed.co_code
restored = changed.replace(co_flags=delegate.co_flags)
assert restored.co_code == ordinary_delegate
for code in (delegate, changed, restored):
    first = code.co_code
    assert list(types.FunctionType(code, {})([1, 2])) == [1, 2]
    assert code.co_code is first

frozen_code = _frozen_importlib._find_spec.__code__
assert _frozen_importlib.__spec__.origin == "frozen"
frozen_bytes = frozen_code.co_code
assert frozen_code.co_code is frozen_bytes

monitoring = sys.monitoring
available = [tool for tool in range(6) if monitoring.get_tool(tool) is None]
assert available
TOOL = available[0]
monitoring.use_tool_id(TOOL, "canonical-code-storage")
try:
    for event in (monitoring.events.LINE, monitoring.events.INSTRUCTION):
        function = make_function()
        code = function.__code__
        observed = []

        def inspect(actual_code, location):
            assert actual_code is code
            if not observed:
                # This is this code object's first co_code read. A separate
                # compilation supplied expected bytes before monitoring began.
                first = actual_code.co_code
                assert first == expected
                assert actual_code.co_code is first
                replacement = actual_code.replace()
                assert replacement.co_code == expected
                assert replacement.co_code is replacement.co_code
                assert types.FunctionType(replacement, {})([3, 4]) == 7
                observed.append(first)

        monitoring.register_callback(TOOL, event, inspect)
        monitoring.set_local_events(TOOL, code, event)
        try:
            assert function([1, 2, 3]) == 6
            assert observed
            assert code.co_code is observed[0]
        finally:
            monitoring.set_local_events(TOOL, code, 0)
            monitoring.register_callback(TOOL, event, None)
        assert code.co_code is observed[0]
        assert function([4, 5]) == 9
finally:
    monitoring.set_events(TOOL, 0)
    monitoring.free_tool_id(TOOL)
