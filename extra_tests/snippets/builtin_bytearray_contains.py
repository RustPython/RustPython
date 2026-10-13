from testutils import assert_raises

for value in (b"", b"a", b"ab\x00\xff"):
    target = bytearray(value)
    for needle in (b"", b"a", b"ab", b"z", b"\x00", b"\xff", 0, 97, 255):
        assert (needle in target) == (needle in value)
    assert target in target


def must_raise_buffer_error(action):
    try:
        action()
    except BufferError:
        return
    raise AssertionError("resize was not rejected")


target = bytearray(b"a")
view = memoryview(target)
try:
    assert b"a" in target
    assert 97 in target
    must_raise_buffer_error(lambda: target.append(0))
    target[:] = b"b"
    assert b"b" in target
    assert 98 in target
finally:
    view.release()
target.append(0)
assert target == b"b\x00"


class IndexNeedle:
    def __index__(self):
        must_raise_buffer_error(lambda: target.append(0))
        target[0] = 98
        return 98


class BytesNeedle(bytes):
    __index__ = IndexNeedle.__index__


for needle in (IndexNeedle(), BytesNeedle(b"\xdd")):
    target[:] = b"a"
    assert needle in target
    assert target == b"b"
    target.append(0)
    assert target == b"b\x00"

for needle in (-1, 256):
    with assert_raises(ValueError):
        needle in target

events = [0, 0]


class BufferNeedle:
    def __buffer__(self, flags):
        must_raise_buffer_error(lambda: target.append(0))
        target[0] = 98
        events[0] += 1
        return memoryview(b"b")

    def __release_buffer__(self, view):
        must_raise_buffer_error(lambda: target.append(0))
        events[1] += 1


target[:] = b"a"
assert BufferNeedle() in target
assert events == [1, 1]
target.append(0)
assert target == b"b\x00"
