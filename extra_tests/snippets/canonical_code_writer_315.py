"""Actual code fields, writer ownership, and canonical monitored serialization."""

import gc
import marshal
import sys
import types
import weakref

SOURCE = "def sample(a):\n    try:\n        return a + marker\n    except TypeError:\n        return 0\n"
FILENAME = "canonical_code_writer.py"
code = compile(SOURCE, FILENAME, "exec").co_consts[0]
assert code.co_linetable and code.co_exceptiontable and code.co_names

for version in range(7):
    for field_name in ("co_consts", "co_names", "co_linetable", "co_exceptiontable"):
        field = getattr(code, field_name)
        for field_first in (False, True):
            graph = (field, code) if field_first else (code, field)
            decoded = marshal.loads(marshal.dumps(graph, version))
            other, restored = decoded if field_first else reversed(decoded)
            assert getattr(restored, field_name) == field
            assert restored.co_code == code.co_code
            if version >= 3:
                assert getattr(restored, field_name) is other
    value = {"tuple": (1, "item"), "set": {2, 3}, "frozen": frozenset((4, 5))}
    assert marshal.loads(marshal.dumps(value, version)) == value

shared = [1, 2]
restored = marshal.loads(marshal.dumps((shared, shared), 6))
assert restored[0] is restored[1]
cycle = []
cycle.append(cycle)
restored = marshal.loads(marshal.dumps(cycle, 6))
assert restored[0] is restored
cycle.clear()
restored.clear()

# Code and slice back-references cannot be completed by the public reader.
for use_code in (False, True):
    child = []
    owner = code.replace(co_consts=(child,)) if use_code else slice(child, None, None)
    child.append(owner)
    try:
        marshal.dumps(owner, 6)
    except ValueError:
        pass
    else:
        raise AssertionError("incomplete immutable cycle accepted")
    child.clear()
    del owner, child


class Unsupported:
    pass


def failed_write(in_set):
    value = Unsupported()
    reference = weakref.ref(value)
    graph = {value} if in_set else [value]
    try:
        marshal.dumps(graph, 6)
    except ValueError:
        pass
    else:
        raise AssertionError("unsupported value accepted")
    return reference


for in_set in (False, True):
    reference = failed_write(in_set)
    gc.collect()
    assert reference() is None

# Successful reservations must also release their owners when dumps returns.
clone = code.replace()
reference = weakref.ref(clone)
marshal.dumps((clone, clone), 6)
del clone
gc.collect()
assert reference() is None

# Serialization, rather than a preceding getter, must obtain canonical bytes
# during the first monitored execution, including opcodes with inline caches.
LOOP = "def total(values):\n    result = 0\n    for value in values:\n        result += value\n    return result\n"
expected = compile(LOOP, FILENAME, "exec").co_consts[0].co_code
monitoring = sys.monitoring
available = [tool for tool in range(6) if monitoring.get_tool(tool) is None]
assert available
tool = available[0]
monitoring.use_tool_id(tool, "canonical-code-writer")
try:
    for event in (monitoring.events.LINE, monitoring.events.INSTRUCTION):
        namespace = {}
        exec(compile(LOOP, FILENAME, "exec"), namespace)
        function = namespace["total"]
        target = function.__code__
        observed = []

        def inspect(actual, location):
            assert actual is target
            if not observed:
                payload = marshal.dumps(actual, 6)
                restored = marshal.loads(payload)
                assert restored.co_code == expected
                assert types.FunctionType(restored, {})([2, 3]) == 5
                observed.append(payload)

        monitoring.register_callback(tool, event, inspect)
        monitoring.set_local_events(tool, target, event)
        try:
            assert function([1, 2, 3]) == 6
            assert len(observed) == 1
        finally:
            monitoring.set_local_events(tool, target, 0)
            monitoring.register_callback(tool, event, None)
finally:
    monitoring.free_tool_id(tool)
