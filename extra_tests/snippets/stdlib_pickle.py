import _pickle
import io
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
