"""Public adaptive bytes preserve layout without exposing private constant opcodes."""

import dis
import marshal
import opcode
import sys

FILENAME = "adaptive_public_export.py"
CONSTANT_SOURCE = (
    'def warm_const():\n    return ("ordinary constant payload", 123456789)\n'
)
LOOP_SOURCE = "def total(n):\n    result = 0\n    for value in range(n):\n        result += value\n    return result\n"
PUBLIC = set(opcode.opmap.values()) | set(opcode._specialized_opmap.values())


def make(source, name):
    namespace = {}
    exec(compile(source, FILENAME, "exec"), namespace)
    return namespace[name]


def check_layout(code, canonical, offsets):
    adaptive = code._co_code_adaptive
    assert len(adaptive) == len(canonical)
    for offset in offsets:
        assert adaptive[offset] in PUBLIC, (offset, adaptive[offset])
        assert adaptive[offset] not in (219, 220, 221)
        assert adaptive[offset + 1] == canonical[offset + 1]
    assert code.co_code == canonical
    return adaptive


function = make(CONSTANT_SOURCE, "warm_const")
code = function.__code__
canonical = code.co_code
offsets = tuple(i.offset for i in dis.get_instructions(code))
cold = check_layout(code, canonical, offsets)
constant_offset = next(
    i.offset for i in dis.get_instructions(code) if i.opname == "LOAD_CONST"
)
assert cold[constant_offset] == opcode.opmap["LOAD_CONST"]
before = marshal.dumps(code, 6)
for _ in range(1000):
    assert function() == ("ordinary constant payload", 123456789)
assert marshal.dumps(code, 6) == before
adaptive = check_layout(code, canonical, offsets)
constants = [
    i for i in dis.get_instructions(code, adaptive=True) if i.opname == "LOAD_CONST"
]
assert len(constants) == 1
assert constants[0].argval == ("ordinary constant payload", 123456789)
assert adaptive[constants[0].offset] == opcode.opmap["LOAD_CONST"]

loop = make(LOOP_SOURCE, "total")
code = loop.__code__
canonical = code.co_code
offsets = tuple(i.offset for i in dis.get_instructions(code))
for _ in range(1000):
    assert loop(10) == 45
first = check_layout(code, canonical, offsets)
second = check_layout(code, canonical, offsets)
assert first == second
assert any(i.cache_info for i in dis.get_instructions(code))
rows = tuple(dis.get_instructions(code, adaptive=True))
assert tuple(i.offset for i in rows) == offsets
assert any(i.opcode in opcode._specialized_opmap.values() for i in rows)
assert all(first[i.offset] == i.opcode for i in rows)

monitoring = sys.monitoring
available = [tool for tool in range(6) if monitoring.get_tool(tool) is None]
assert available
TOOL = available[0]
monitoring.use_tool_id(TOOL, "adaptive-public-export")
try:
    for event in (monitoring.events.LINE, monitoring.events.INSTRUCTION):
        function = make(LOOP_SOURCE, "total")
        code = function.__code__
        canonical = code.co_code
        offsets = tuple(i.offset for i in dis.get_instructions(code))
        observed = []

        def inspect(actual_code, location):
            assert actual_code is code
            if not observed:
                # First adaptive export for this code happens under active wrappers.
                observed.append(check_layout(actual_code, canonical, offsets))
                assert marshal.loads(marshal.dumps(actual_code, 6)).co_code == canonical

        monitoring.register_callback(TOOL, event, inspect)
        monitoring.set_local_events(TOOL, code, event)
        try:
            assert function(10) == 45
            assert len(observed) == 1
        finally:
            monitoring.set_local_events(TOOL, code, 0)
            monitoring.register_callback(TOOL, event, None)
        check_layout(code, canonical, offsets)
        assert function(5) == 10
finally:
    monitoring.set_events(TOOL, 0)
    monitoring.free_tool_id(TOOL)
