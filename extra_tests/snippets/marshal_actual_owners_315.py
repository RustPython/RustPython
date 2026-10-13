"""Actual-owner reference selection: complete streams and bounded lifetimes."""

import gc
import marshal
import sys
import weakref

# These full certified streams describe an explicitly aliased root and a
# repeated child. Direct/named ownership streams are retained separately; this
# golden does not claim root-transport parity for those other Python paths.
GOLDENS = (
    "28020000005b030000006604312e32357801320133730200000041425b030000006604312e3235780132013373020000004142",
    "28020000005b030000006604312e32357801320133730200000041425b030000006604312e3235780132013373020000004142",
    "28020000005b0300000067000000000000f43f7900000000000000400000000000000840730200000041425b0300000067000000000000f43f790000000000000040000000000000084073020000004142",
    "a802000000db0300000067000000000000f43f7900000000000000400000000000000840730200000041427201000000",
    "a902db0300000067000000000000f43f7900000000000000400000000000000840730200000041427201000000",
    "a902db0300000067000000000000f43f7900000000000000400000000000000840730200000041427201000000",
    "a902db0300000067000000000000f43f7900000000000000400000000000000840730200000041427201000000",
)


def make_graph():
    child = [float("1.25"), complex("2+3j"), bytes(bytearray([65, 66]))]
    return (child, child)


for version, expected in enumerate(GOLDENS):
    graph = make_graph()
    aliases = [graph, graph]
    payload = marshal.dumps(graph, version)
    assert aliases[0] is aliases[1] is graph
    assert payload == bytes.fromhex(expected)
    restored = marshal.loads(payload)
    assert restored == graph
    if version >= 3:
        assert restored[0] is restored[1]

for version in range(7):
    text = sys.intern("".join(("ordinary", "_interned", "_owner")))
    objects = {
        "dict": {"value": [1, 2], "other": (3, 4)},
        "set": {1.25, 2.75},
        "frozen": frozenset((1.25, 2.75)),
        "interned": (text, text),
    }
    restored = marshal.loads(marshal.dumps(objects, version))
    assert restored == objects
    if version >= 3:
        assert restored["interned"][0] is restored["interned"][1]

for version in range(3, 7):
    child = []
    graph = {"first": child, "second": child}
    child.append(graph)
    restored = marshal.loads(marshal.dumps(graph, version))
    assert restored["first"] is restored["second"]
    assert restored["first"][0] is restored
    child.clear()
    restored["first"].clear()


class Unsupported:
    pass


def failed_writer(in_set):
    value = Unsupported()
    reference = weakref.ref(value)
    graph = {value} if in_set else {"value": value}
    try:
        marshal.dumps(graph, 6)
    except ValueError:
        pass
    else:
        raise AssertionError("unsupported value accepted")
    return reference


for in_set in (False, True):
    for _ in range(8):
        reference = failed_writer(in_set)
        gc.collect()
        assert reference() is None

# Actual weak-reference acquisition and release must not leak a writer owner.
# This checks bounded lifetime/acceptance, not an exclusive-count guarantee.
code = compile("pass", "ordinary_owner.py", "exec")
reference = weakref.ref(code)
for _ in range(32):
    acquired = reference()
    assert acquired is code
    marshal.loads(marshal.dumps((code, acquired), 6))
    del acquired
try:
    marshal.dumps(code, 6, allow_code=False)
except ValueError:
    pass
else:
    raise AssertionError("disallowed code accepted")
del code
gc.collect()
assert reference() is None

# Observational stress only: scheduling need not overlap, and this does not
# establish snapshot semantics or CPython free-threaded count equivalence.
import _thread

code = compile("value = 7", "ordinary_owner.py", "exec")
reference = weakref.ref(code)
done = _thread.allocate_lock()
done.acquire()
errors = []


def acquire_owners():
    try:
        for _ in range(512):
            value = reference()
            assert value is not None
            del value
    except BaseException as error:
        errors.append(error)
    finally:
        done.release()


_thread.start_new_thread(acquire_owners, ())
for _ in range(64):
    restored = marshal.loads(marshal.dumps(code, 6))
    assert restored.co_code == code.co_code
assert done.acquire(timeout=10)
done.release()
assert not errors
del code, restored
gc.collect()
assert reference() is None
