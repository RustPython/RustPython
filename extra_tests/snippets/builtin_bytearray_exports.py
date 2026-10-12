"""Resizing paths must respect outstanding bytearray exports."""

import io

from testutils import assert_raises

for args in ((), (b"abc",), (b"x",), (b"",), (object(),)):
    value = bytearray(b"abc")
    with memoryview(value) as view:
        with assert_raises(BufferError):
            value.__init__(*args)
        assert value == b"abc"
        assert view.tobytes() == b"abc"
    value.__init__(b"released")
    assert value == b"released"

value = bytearray(b"abc")
with assert_raises(TypeError):
    value.__init__(object())
assert value == b""
value = bytearray(b"abc")
value.__init__(value)
assert value == b""


class ExportDuringInit:
    def __init__(self, count, payload=b""):
        self.count = count
        self.payload = payload
        self.calls = 0

    def __index__(self):
        self.calls += 1
        value.extend(self.payload)
        self.view = memoryview(value)
        return self.count


value = bytearray(b"abc")
source = ExportDuringInit(2)
with assert_raises(BufferError):
    value.__init__(source)
assert value == b""
assert source.view.tobytes() == b""
assert source.calls == 1
source.view.release()
value.extend(b"released")
assert value == b"released"

value = bytearray(b"abc")
source = ExportDuringInit(0)
value.__init__(source)
assert value == b""
assert source.view.tobytes() == b""
source.view.release()

value = bytearray(b"abc")
source = ExportDuringInit(0, b"xy")
value.__init__(source)
assert value == b"xy"
assert source.view.tobytes() == b"xy"
assert source.calls == 1
source.view.release()


class FailingIndex:
    def __index__(self):
        assert value == b""
        raise ValueError("conversion failed")


value = bytearray(b"abc")
with assert_raises(ValueError):
    value.__init__(FailingIndex())
assert value == b""

for key in (0, slice(0, 1)):
    value = bytearray(b"abc")
    with memoryview(value) as view:
        with assert_raises(BufferError):
            del value[key]
        assert view.tobytes() == b"abc"
    del value[key]
    assert value == b"bc"


class Reader(io.RawIOBase):
    def __init__(self, count, retain, payload=b"abcd"):
        self.count = count
        self.retain = retain
        self.payload = payload

    def readinto(self, buffer):
        buffer[:] = self.payload
        self.buffer = buffer
        if self.retain:
            self.view = memoryview(buffer)
        return self.count


for count in (0, 2, 4):
    reader = Reader(count, False)
    assert reader.read(4) == b"abcd"[:count]
    assert reader.buffer == b""

    reader = Reader(count, True)
    with assert_raises(BufferError):
        reader.read(4)
    assert reader.view.tobytes() == b"abcd"
    reader.view.release()
    reader.buffer.clear()

for count in (-1, 5):
    reader = Reader(count, True)
    with assert_raises(ValueError):
        reader.read(4)
    assert reader.view.tobytes() == b"abcd"
    reader.view.release()

reader = Reader(None, True)
assert reader.read(4) is None
assert reader.view.tobytes() == b"abcd"
reader.view.release()

# The callback can shrink the temporary buffer, so slicing by the original
# requested size without checking the current buffer is not safe.
reader = Reader(2, False, b"xy")
assert reader.read(4) == b"xy"
assert reader.buffer == b""

reader = Reader(4, False, b"xy")
result = reader.read(4)
assert len(result) == 4
assert result[:2] == b"xy"
# The contents of the newly grown region are unspecified by CPython.
assert reader.buffer == b""
