"""Stable surviving code fields and sole constants tuple ownership."""

import gc
import marshal
import sys
import types
import weakref

SOURCE = "def sample(a):\n    try:\n        return a + marker\n    except TypeError:\n        return 0\n"
FILENAME = "stable_code_fields.py"
base = compile(SOURCE, FILENAME, "exec").co_consts[0]
for name in (
    "co_consts",
    "co_names",
    "co_varnames",
    "co_freevars",
    "co_cellvars",
    "co_linetable",
    "co_exceptiontable",
    "co_code",
):
    value = getattr(base, name)
    assert getattr(base, name) is value

replacement = base.replace()
for name in ("co_consts", "co_names", "co_linetable", "co_exceptiontable"):
    assert getattr(replacement, name) is getattr(base, name)
assert replacement.co_code == base.co_code
assert replacement.co_code is not base.co_code
assert replacement.co_varnames == base.co_varnames
assert replacement.co_varnames is not base.co_varnames


class Child:
    pass


child = Child()
weak = weakref.ref(child)
values = (child, [1, 2], slice(1, 5, 2), frozenset((3, 4)))
owned = base.replace(co_consts=values)
assert owned.co_consts is values
assert owned.co_consts[0] is child
assert owned.co_consts[2] is values[2]
assert owned.co_consts[3] is values[3]
clone = owned.replace()
assert clone.co_consts is values
del child, owned, values
gc.collect()
assert weak() is not None
del clone
gc.collect()
assert weak() is None

# The public constructor copies validated names, retains its constants and
# surviving byte tables, and independently creates its exported code cache.
input_code = bytes(bytearray(base.co_code))
input_names = tuple(list(base.co_names))
constructed = types.CodeType(
    base.co_argcount,
    base.co_posonlyargcount,
    base.co_kwonlyargcount,
    base.co_nlocals,
    base.co_stacksize,
    base.co_flags,
    input_code,
    base.co_consts,
    input_names,
    base.co_varnames,
    FILENAME,
    base.co_name,
    base.co_qualname,
    base.co_firstlineno,
    base.co_linetable,
    base.co_exceptiontable,
    base.co_freevars,
    base.co_cellvars,
)
assert constructed.co_consts is base.co_consts
assert constructed.co_names == input_names
assert constructed.co_names is not input_names
assert constructed.co_linetable is base.co_linetable
assert constructed.co_exceptiontable is base.co_exceptiontable
assert constructed.co_code == input_code
assert constructed.co_code is not input_code

# Retain the known normalization gap. Rust never mutates a published caller
# tuple; when a member needs canonical interning it creates a normalized copy.
raw_name = "".join(("mar", "ker"))
assert raw_name is not base.co_names[0]
names = (raw_name,)
normalized = base.replace(co_names=names)
assert normalized.co_names[0] is base.co_names[0]
if sys.implementation.name == "rustpython":
    assert normalized.co_names is not names
    assert names[0] is raw_name
else:
    assert normalized.co_names is names
    assert names[0] is base.co_names[0]

outer = compile(
    "def outer(a):\n    def inner():\n        return a\n    return inner\n",
    FILENAME,
    "exec",
).co_consts[0]
inner = next(value for value in outer.co_consts if isinstance(value, types.CodeType))
assert outer.co_cellvars == ("a",)
assert outer.co_cellvars is outer.co_cellvars
assert inner.co_freevars == ("a",)
assert inner.co_freevars is inner.co_freevars

# Whole unchanged streams emitted by the pinned final oracle. Both field-first
# and code-first ordering exercise reader reference identity; the current Rust
# writer is deliberately not used to create the expected aliases.
FIELD_FIRST = bytes.fromhex(
    "2905a901e900000000a902da066d61726b6572da09547970654572726f72f3230000008100f002030511d80f"
    "1094368d7ad00819f8dc0b14f400010511da0f10f003010511faf310000000830c100090091f039a011f039e"
    "011f03e30100000000000000000000000400000003000000f34400000080000000190055005b000000000000"
    "0000002a000000000000000000000021001e005b0200000000000000000600630500001a001d001b005d0021"
    "00680039031b006801720000000072020000002901da0161730100000026da15737461626c655f636f64655f"
    "6669656c64732e7079da0673616d706c65720b0000000100000072050000007206000000"
)
CODE_FIRST = bytes.fromhex(
    "2905e30100000000000000000000000400000003000000f34400000080000000190055005b00000000000000"
    "00002a000000000000000000000021001e005b0200000000000000000600630500001a001d001b005d002100"
    "680039031b006801a901e900000000a902da066d61726b6572da09547970654572726f722901da0161730100"
    "000026da15737461626c655f636f64655f6669656c64732e7079da0673616d706c65720900000001000000f3"
    "230000008100f002030511d80f1094368d7ad00819f8dc0b14f400010511da0f10f003010511faf310000000"
    "830c100090091f039a011f039e011f0372020000007204000000720a000000720b000000"
)
for order, payload in (("field_first", FIELD_FIRST), ("code_first", CODE_FIRST)):
    decoded = marshal.loads(payload)
    if order == "field_first":
        consts, names, lines, exceptions, code = decoded
    else:
        code, consts, names, lines, exceptions = decoded
    assert code.co_consts is consts
    assert code.co_names is names
    assert code.co_linetable is lines
    assert code.co_exceptiontable is exceptions
    assert code.co_code == base.co_code
    assert code.co_code is code.co_code
    function = types.FunctionType(code, {"marker": 10})
    assert function(2) == 12
    assert function("x") == 0

# Final CodeType.replace accepts tuple fields, not arbitrary iterables.
for field in ("co_consts", "co_names"):
    try:
        base.replace(**{field: []})
    except TypeError:
        pass
    else:
        raise AssertionError(field + " accepted a list")
