import _pickle
import io

# BINPUT leaves holes; MEMOIZE uses the number of present entries.
unpickler = _pickle.Unpickler(io.BytesIO(b"\x80\x04]q\x07\x94h\x01\x86."))
left, right = unpickler.load()
assert left is right
memo = unpickler.memo.copy()
assert list(memo) == [1, 7]
assert memo[1] is left
assert memo[7] is left
