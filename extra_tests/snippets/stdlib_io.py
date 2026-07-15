import os
from io import BufferedReader, BytesIO, FileIO, RawIOBase, StringIO, TextIOWrapper

from testutils import assert_raises

fi = FileIO("README.md")
assert fi.seekable()
bb = BufferedReader(fi)
assert bb.seekable()

result = bb.read()

assert len(result) <= 16 * 1024
assert len(result) >= 0
assert isinstance(result, bytes)

with FileIO("README.md") as fio:
    res = fio.read()
    assert len(res) <= 16 * 1024
    assert len(res) >= 0
    assert isinstance(res, bytes)

fd = os.open("README.md", os.O_RDONLY)

with FileIO(fd) as fio:
    res2 = fio.read()
    assert res == res2

fi = FileIO("README.md")
fi.read()
fi.close()
assert fi.closefd
assert fi.closed

with assert_raises(ValueError):
    fi.read()

with FileIO("README.md") as fio:
    nres = fio.read(1)
    assert len(nres) == 1
    nres = fio.read(2)
    assert len(nres) == 2


# Test that IOBase.isatty() raises ValueError when called on a closed file.
# Minimal subclass that inherits IOBase.isatty() without overriding it.
class MinimalRaw(RawIOBase):
    def readinto(self, b):
        return 0


f = MinimalRaw()
assert not f.closed
assert not f.isatty()  # open file: should return False

f.close()
assert f.closed

with assert_raises(ValueError):
    f.isatty()


class Gh6588:
    def __init__(self):
        self.textio = None
        self.closed = False

    def writable(self):
        return True

    def readable(self):
        return False

    def seekable(self):
        return False

    def write(self, data):
        self.textio.reconfigure(encoding="utf-8")
        return len(data)


raw = Gh6588()
textio = TextIOWrapper(raw, encoding="utf-8", write_through=True)
raw.textio = textio
with assert_raises(AttributeError):
    textio.writelines(["x"])

textio = TextIOWrapper(BytesIO())

for invalid_chunk_size in (0, -1, 2**100):
    try:
        textio._CHUNK_SIZE = invalid_chunk_size
    except ValueError:
        pass
    else:
        raise AssertionError(f"expected ValueError for {invalid_chunk_size!r}")

for invalid_chunk_size in (1.5, "4"):
    try:
        textio._CHUNK_SIZE = invalid_chunk_size
    except TypeError:
        pass
    else:
        raise AssertionError(f"expected TypeError for {invalid_chunk_size!r}")


class ChunkSize:
    def __index__(self):
        return 16


textio._CHUNK_SIZE = ChunkSize()
assert textio._CHUNK_SIZE == 16


class OversizedChunkSize:
    def __index__(self):
        return 2**100


try:
    textio._CHUNK_SIZE = OversizedChunkSize()
except ValueError as error:
    expected = "cannot fit 'OversizedChunkSize' into an index-sized integer"
    if str(error) != expected:
        raise AssertionError(f"unexpected error message: {error}") from error
else:
    raise AssertionError("expected ValueError for oversized indexable object")
