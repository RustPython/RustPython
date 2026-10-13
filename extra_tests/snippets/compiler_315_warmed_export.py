"""Ordinary warmed exports retain public opcodes and unchanged raw fields."""

import marshal
import sys
import types

if sys.version_info < (3, 15):
    sys.exit(0)


source = """
def direct(x):
    return x + 1000
def looping(xs):
    total = 0
    for x in xs:
        total += direct(x)
    return total
def delegate(xs):
    yield from xs
"""
root = compile(source, "byte_exact/warmed_export.py", "exec")
namespace = {}
exec(root, namespace)
codes = [value for value in root.co_consts if isinstance(value, types.CodeType)]
before = [(code.co_code, code.co_linetable, code.co_exceptiontable) for code in codes]
for _ in range(100):
    assert namespace["looping"]([1, 2, 3]) == 3006
    assert list(namespace["delegate"]([1, 2])) == [1, 2]
after = [(code.co_code, code.co_linetable, code.co_exceptiontable) for code in codes]
assert before == after

# Generated cache widths in final 3.15 pycore_opcode_metadata.h. Operand bytes
# are not opcode positions and must not be rejected by a byte-membership test.
cache_widths = {
    128: 1,
    37: 3,
    36: 1,
    105: 1,
    68: 1,
    118: 1,
    109: 4,
    91: 4,
    95: 1,
    79: 9,
    54: 1,
    55: 1,
    74: 1,
    102: 1,
    99: 1,
    100: 1,
    101: 1,
    70: 1,
    50: 3,
    53: 3,
    4: 1,
    42: 5,
}
for code in codes:
    exported = code.co_code
    index = 0
    while index < len(exported):
        opcode = exported[index]
        assert opcode not in (219, 220, 221)
        index += 2 * (1 + cache_widths.get(opcode, 0))
    assert index == len(exported)
    loaded = marshal.loads(marshal.dumps(code, 6))
    assert loaded.co_code == exported
    assert loaded.co_linetable == code.co_linetable
    assert loaded.co_exceptiontable == code.co_exceptiontable
