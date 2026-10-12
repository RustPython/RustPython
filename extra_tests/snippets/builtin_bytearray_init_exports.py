"""Initializer storage and integer-deletion ordering with buffer exports."""

from testutils import assert_raises


class Zero:
    def __init__(self, value):
        self.value = value
        self.observations = []

    def __index__(self):
        self.observations.append((len(self.value), self.value.__alloc__()))
        return 0


# Final CPython rejects even an empty exported initializer before conversion.
for kind in ("missing", "integer", "callback"):
    value = bytearray()
    source = Zero(value)
    with memoryview(value) as view:
        with assert_raises(BufferError):
            if kind == "missing":
                value.__init__()
            elif kind == "integer":
                value.__init__(0)
            else:
                value.__init__(source)
        assert value == b""
        assert view.tobytes() == b""
        assert source.observations == []

# Clearing an initializer must release its old storage before callbacks and
# before the successful zero-count early return.
for kind in ("missing", "integer", "callback"):
    value = bytearray(b"x" * 64)
    source = Zero(value)
    assert value.__alloc__() > 0
    if kind == "missing":
        value.__init__()
    elif kind == "integer":
        value.__init__(0)
    else:
        value.__init__(source)
        assert source.observations == [(0, 0)]
    assert value == b""
    assert value.__alloc__() == 0

# An invalid integer index is rejected before checking whether resizing is
# blocked, and neither error path may alter the exported bytes.
for initial, indices in ((b"abc", (3, -4)), (b"", (0, -1))):
    for index in indices:
        value = bytearray(initial)
        with memoryview(value) as view:
            try:
                del value[index]
            except IndexError as exc:
                assert str(exc) == "bytearray index out of range"
            else:
                raise AssertionError("invalid index was accepted")
            assert value == initial
            assert view.tobytes() == initial

for index in (0, -1):
    value = bytearray(b"abc")
    with memoryview(value) as view:
        with assert_raises(BufferError):
            del value[index]
        assert view.tobytes() == b"abc"
    del value[index]
    assert value == (b"bc" if index == 0 else b"ab")
