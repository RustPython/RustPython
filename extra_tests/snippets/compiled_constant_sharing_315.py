"""Compilation-local field identities, with actual complete-stream witnesses.

This does not claim full compiler first-winner chronology or native root parity.
NaN/code/slice key exclusions are tested structurally on the Rust Basic route.
"""

import ast
import gc
import marshal
import types
import weakref

SOURCE = """def left(x):
    try:
        return x + 12345678901234567890
    except TypeError:
        return ('shared payload !', b'shared bytes !', (98765432109876543210,))
def right(x):
    try:
        return x + 12345678901234567890
    except TypeError:
        return ('shared payload !', b'shared bytes !', (98765432109876543210,))
def outer():
    def inner(x):
        return x + 12345678901234567890
    return inner
def member_left(x):
    return x in {12345678901234567890, 98765432109876543210}
def member_right(x):
    return x in {12345678901234567890, 98765432109876543210}
def import_scope():
    import ordinary_name
"""


def children(code):
    return {
        value.co_name: value
        for value in code.co_consts
        if isinstance(value, types.CodeType)
    }


def constant(code, kind):
    values = [value for value in code.co_consts if type(value) is kind]
    assert len(values) == 1, (kind, values)
    return values[0]


def check_compile(source):
    root = compile(source, "compile_sharing.py", "exec")
    codes = children(root)
    left, right = codes["left"], codes["right"]
    assert left is not right
    for name in ("co_consts", "co_names", "co_linetable", "co_exceptiontable"):
        first, second = getattr(left, name), getattr(right, name)
        assert first, name  # Every tested owner field is nonempty.
        assert first is second, name
    assert left.co_varnames == right.co_varnames == ("x",)
    assert left.co_varnames is not right.co_varnames
    assert left.co_code == right.co_code
    assert left.co_code is not right.co_code
    inner = children(codes["outer"])["inner"]
    assert constant(left, int) is constant(inner, int)
    first_set = constant(codes["member_left"], frozenset)
    second_set = constant(codes["member_right"], frozenset)
    assert first_set is second_set
    assert any(value is constant(left, int) for value in first_set)
    # Derived varnames are separately cached, even when the internal names and
    # localsplusnames tuple owners can be the same compiler cache winner.
    imported = codes["import_scope"]
    assert imported.co_names == imported.co_varnames == ("ordinary_name",)
    assert imported.co_names is not imported.co_varnames
    for version in range(3, 7):
        stream = marshal.dumps(
            [left, right, left.co_consts, left.co_linetable], version
        )
        decoded = marshal.loads(stream)
        assert decoded[0].co_consts is decoded[1].co_consts is decoded[2]
        assert decoded[0].co_linetable is decoded[1].co_linetable is decoded[3]
    return root


source_root = check_compile(SOURCE)
ast_root = check_compile(ast.parse(SOURCE))
source_left = children(source_root)["left"]
ast_left = children(ast_root)["left"]
assert source_left.co_consts == ast_left.co_consts
assert source_left.co_consts is not ast_left.co_consts
assert constant(source_left, int) is not constant(ast_left, int)
assert source_left.co_linetable == ast_left.co_linetable
assert source_left.co_linetable is not ast_left.co_linetable

# Safe owner release: an externally retained constants tuple outlives its code,
# and does not itself retain the code. No cyclic-GC policy is asserted.
retained = source_left.co_consts
reference = weakref.ref(source_left)
del source_left, source_root
gc.collect()
assert reference() is None
assert 12345678901234567890 in retained

# Scalar typed and signed-zero distinctions remain intact in source compilation.
scalar = compile("value = (False, 0, 0.0, -0.0, 0j, -0j)", "compile_sharing.py", "exec")
values = constant(scalar, tuple)
assert [type(value) for value in values] == [bool, int, float, float, complex, complex]
assert values[2] is not values[3]
assert values[4] is not values[5]
assert marshal.dumps(values[2]) != marshal.dumps(values[3])
assert marshal.dumps(values[4]) != marshal.dumps(values[5])

# Public construction/replace are not a new compilation session. Retain their
# actual supplied tuple owners rather than pooling equal values globally.
base = children(ast_root)["left"]
first = tuple([None, bytes(bytearray(b"public constructor bytes !"))])
second = tuple([None, bytes(bytearray(b"public constructor bytes !"))])
a = base.replace(co_consts=first)
b = base.replace(co_consts=second)
assert a.co_consts is first and b.co_consts is second
assert a.co_consts == b.co_consts and a.co_consts is not b.co_consts
assert base.co_consts is not first

# AST constants retain nonfinite values. Distinct NaN values keep their own
# aggregates while ordinary equal byte siblings can still share within the pool.
for make_nan in (
    lambda: float("nan"),
    lambda: complex(float("nan"), 1),
    lambda: complex(1, float("nan")),
):
    left_value = (make_nan(), bytes(bytearray(b"safe sibling next to NaN")))
    right_value = (make_nan(), bytes(bytearray(b"safe sibling next to NaN")))
    assert left_value[1] is not right_value[1]
    tree = ast.Expression(body=ast.Constant(value=(left_value, right_value)))
    nan_code = compile(ast.fix_missing_locations(tree), "compile_sharing.py", "eval")
    left_value, right_value = constant(nan_code, tuple)
    assert left_value is not right_value
    assert left_value[0] is not right_value[0]
    assert left_value[1] is right_value[1]
    for version in range(7):
        restored = marshal.loads(marshal.dumps((left_value, right_value), version))
        assert type(restored[0][0]) is type(left_value[0])
        assert restored[0][1] == restored[1][1] == b"safe sibling next to NaN"
        if version >= 3:
            assert restored[0][1] is restored[1][1]

infinities = (
    float("inf"),
    float("-inf"),
    complex(float("inf"), float("-inf")),
    complex(float("-inf"), float("inf")),
)
# Build equal but independently allocated input objects to exercise compilation.
repeated = tuple(type(value)(str(value)) for value in infinities)
assert all(a is not b for a, b in zip(infinities, repeated))
tree = ast.Expression(body=ast.Constant(value=infinities + repeated))
inf_code = compile(ast.fix_missing_locations(tree), "compile_sharing.py", "eval")
values = constant(inf_code, tuple)
assert values[:4] == values[4:]
assert all(a is b for a, b in zip(values[:4], values[4:]))
assert len({id(value) for value in values}) == 4
