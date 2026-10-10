import _pickle
import io
from types import MappingProxyType

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
replacement_getattr = object()
for builtins_type in (dict, MappingProxyType):
    namespace = {
        "__builtins__": builtins_type({"getattr": replacement_getattr}),
        "descriptor": object.__str__,
    }
    exec("reduced = descriptor.__reduce__()", namespace)
    assert namespace["reduced"] == (replacement_getattr, (object, "__str__"))

    namespace["__builtins__"] = builtins_type({})
    with assert_raises(AttributeError) as caught:
        exec("descriptor.__reduce__()", namespace)
    assert caught.exception.args == ("getattr",)
