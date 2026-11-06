import _pickle
import gc
import io
import sys
import weakref
from types import MappingProxyType, MethodType

from testutils import assert_raises

# BINPUT leaves holes; MEMOIZE uses the number of present entries.
unpickler = _pickle.Unpickler(io.BytesIO(b"\x80\x04]q\x07\x94h\x01\x86."))
left, right = unpickler.load()
assert left is right
memo = unpickler.memo.copy()
assert list(memo) == [1, 7]
assert memo[1] is left
assert memo[7] is left

# Slot wrapper descriptors reduce to lookup on their defining class.
assert object.__str__.__reduce__() == (getattr, (object, "__str__"))
assert _pickle.loads(_pickle.dumps(object.__str__)) is object.__str__

# Oversized advertised lengths are rejected before trying to read or allocate.
for opcode, name in (
    (b"\x8e", "BINBYTES"),
    (b"\x8d", "BINUNICODE"),
    (b"\x95", "FRAME"),
):
    data = b"\x80\x04" + opcode + (sys.maxsize + 1).to_bytes(8, "little")
    for load in (
        _pickle.loads,
        lambda data: _pickle.Unpickler(io.BytesIO(data)).load(),
    ):
        with assert_raises(OverflowError) as caught:
            load(data)
        if name == "FRAME":
            expected = f"FRAME length exceeds system's maximum of {sys.maxsize} bytes"
        else:
            expected = f"{name} exceeds system's maximum size of {sys.maxsize} bytes"
        assert str(caught.exception) == expected


# Reduction returns getattr from the active frame's builtins without calling it.
class ReductionOwner:
    __slots__ = ("slot",)

    def method(self):
        pass


owner = ReductionOwner()
items = []
replacement_getattr = object()
for descriptor, receiver, name in (
    (object.__str__, object, "__str__"),
    (str.join, str, "join"),
    (io.BytesIO.getvalue, io.BytesIO, "getvalue"),
    (ReductionOwner.slot, ReductionOwner, "slot"),
    (owner.__str__, owner, "__str__"),
    (items.append, items, "append"),
    (dict.fromkeys, dict, "fromkeys"),
    (owner.method, owner, "method"),
):
    for builtins_type in (dict, MappingProxyType):
        namespace = {
            "__builtins__": builtins_type({"getattr": replacement_getattr}),
            "descriptor": descriptor,
        }
        exec("reduced = descriptor.__reduce__()", namespace)
        assert namespace["reduced"] == (replacement_getattr, (receiver, name))

        namespace["__builtins__"] = builtins_type({})
        with assert_raises(AttributeError) as caught:
            exec("descriptor.__reduce__()", namespace)
        assert caught.exception.args == ("getattr",)

# Module-bound functions reduce to a name without looking up getattr.
namespace = {"__builtins__": {}, "function": len}
exec("reduced = function.__reduce__()", namespace)
assert namespace["reduced"] == "len"


class MissingBuiltin(dict):
    def __getitem__(self, key):
        raise KeyError(key)

    def __missing__(self, key):
        raise AssertionError("must not retry __missing__ after custom __getitem__")


for descriptor, name in (
    (object.__str__, "getattr"),
    (iter([1]), "iter"),
    (reversed([1]), "reversed"),
):
    namespace = {"__builtins__": MissingBuiltin(), "descriptor": descriptor}
    with assert_raises(AttributeError) as caught:
        exec("descriptor.__reduce__()", namespace)
    assert caught.exception.args == (name,)


name_error = RuntimeError("function name first")


class NamedCallable:
    def __call__(self, *args):
        pass

    @property
    def __name__(self):
        raise name_error


namespace = {
    "__builtins__": {},
    "method": MethodType(NamedCallable(), owner),
}
with assert_raises(RuntimeError) as caught:
    exec("method.__reduce__()", namespace)
assert caught.exception is name_error


# Unpickler payloads can retain a cycle through their memo proxy.
class UnpicklerCycleStream:
    def __init__(self):
        self.source = io.BytesIO(_pickle.dumps([1, 2, 3], protocol=4))
        self.read = self.source.read
        self.readline = self.source.readline
        self.peek = self.peek_bytes

    def peek_bytes(self, size=0):
        position = self.source.tell()
        data = self.source.read(size)
        self.source.seek(position)
        return data


class UnpicklerCycleBuffers:
    def __iter__(self):
        return self

    def __next__(self):
        raise StopIteration


def unpickler_payload_cycle(kind):
    stream = UnpicklerCycleStream()
    buffers = UnpicklerCycleBuffers()
    unpickler = _pickle.Unpickler(stream, buffers=buffers)
    holder = stream if kind == "stream" else buffers
    holder.proxy = unpickler.memo
    return weakref.ref(holder)


for kind in ("stream", "buffers"):
    reference = unpickler_payload_cycle(kind)
    for _ in range(3):
        gc.collect()
    assert reference() is None, kind
