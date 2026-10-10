import _pickle
import io
import sys

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
