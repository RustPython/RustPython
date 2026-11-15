import mmap

from testutils import assert_raises

mapped = mmap.mmap(-1, 1)
assert mapped.seekable()
mapped.close()
assert mapped.seekable()

mapped = mmap.mmap(-1, 10)
# an inverted range finds nothing rather than being subtracted into a huge one
assert mapped.find(b"x", 5, 2) == -1
assert mapped.rfind(b"x", 5, 2) == -1
# both offsets are bounds-checked before anything is copied
with assert_raises(ValueError):
    mapped.move(20, 0, 1)
with assert_raises(ValueError):
    mapped.move(0, 20, 1)
mapped.close()

mapped = mmap.mmap(-1, 3)
mapped[:] = b"abc"
# the empty subsequence matches at the edge each scan starts from
assert mapped.find(b"") == 0
assert mapped.rfind(b"") == 3
assert mapped.find(b"", 1, 2) == 1
assert mapped.rfind(b"", 1, 2) == 2
# starts past the end find nothing and negative offsets count from the end
assert mapped.find(b"", 5) == -1
assert mapped.rfind(b"", 5) == -1
assert mapped.find(b"", -1) == 2
assert mapped.rfind(b"", 0, -1) == 2
# an inverted range holds nothing, not even the empty subsequence
assert mapped.find(b"", 2, 1) == -1
assert mapped.rfind(b"", 2, 1) == -1
mapped.close()
# a closed mmap is rejected before the subsequence is inspected
with assert_raises(ValueError):
    mapped.find(b"")
with assert_raises(ValueError):
    mapped.rfind(b"")


# Buffer acquisition must reject a closed mapping before returning a view.
with assert_raises(ValueError):
    memoryview(mapped)

mapped = mmap.mmap(-1, 4)
mapped[:] = b"data"
view = memoryview(mapped)
child = view[1:]
view.release()
with assert_raises(BufferError):
    mapped.close()
assert not mapped.closed
assert child.tobytes() == b"ata"
child.release()
mapped.close()
mapped.close()

# On Windows resize must neither invalidate an export nor revive a closed map.
import sys

if sys.platform == "win32":
    mapped = mmap.mmap(-1, 4)
    mapped[:] = b"data"
    with memoryview(mapped) as view:
        with assert_raises(BufferError):
            mapped.resize(8)
        with assert_raises(BufferError):
            mapped.resize(0)
        assert view.tobytes() == b"data"
    mapped.resize(8)
    with memoryview(mapped) as view:
        assert view.tobytes() == b"data" + b"\0" * 4
    mapped.close()
    with assert_raises(ValueError):
        mapped.resize(4)
    assert mapped.closed


# Assignment validates current storage after argument conversion, even if empty.
with mmap.mmap(-1, 4) as mapped:
    with assert_raises(ValueError):
        mapped[4] = 256
    mapped[0:0] = b""
with assert_raises(ValueError):
    mapped[0:0] = b""

with mmap.mmap(-1, 4, access=mmap.ACCESS_READ) as mapped:
    with assert_raises(TypeError):
        mapped[0:0] = b""


class CloseOnIndex:
    def __init__(self, mapped):
        self.mapped = mapped

    def __index__(self):
        self.mapped.close()
        return 0


for close_from in ("key", "value", "slice"):
    mapped = mmap.mmap(-1, 4)
    closer = CloseOnIndex(mapped)
    try:
        with assert_raises(ValueError):
            if close_from == "key":
                mapped[closer] = 0
            elif close_from == "value":
                mapped[0] = closer
            else:
                mapped[closer:0] = b""
        assert mapped.closed
    finally:
        mapped.close()


class FailingIndex:
    def __index__(self):
        raise RuntimeError


# CPython parses mmap() with PyArg_ParseTupleAndKeywords, which converts fileno
# before it reports the missing length.
with assert_raises(RuntimeError):
    mmap.mmap(FailingIndex())


# resize uses Clinic's index conversion before checking the mapping. Unlike
# read/write past a resized end, tell() retains the original cursor.
if hasattr(mmap.mmap, "resize"):

    class ResizeIndex:
        def __index__(self):
            return 8

    with mmap.mmap(-1, 16) as mapped:
        mapped.seek(12)
        mapped.resize(ResizeIndex())
        assert len(mapped) == 8
        assert mapped.tell() == 12
        assert mapped.read() == b""
        with assert_raises(OverflowError):
            mapped.resize(1 << 100)
        with assert_raises(ValueError):
            mapped.resize(-1)

    mapped = mmap.mmap(-1, 16)
    with assert_raises(ValueError):
        mapped.resize(CloseOnIndex(mapped))
    assert mapped.closed


import os

if os.name == "nt":
    import tempfile
    import uuid

    # Named file mappings retain the file handle, not just the view length.
    with tempfile.TemporaryFile() as backing:
        backing.write(b"x" * 32)
        backing.flush()
        tag = str(uuid.uuid4())
        backing.seek(5)
        with mmap.mmap(backing.fileno(), 8, tagname=tag) as mapped:
            assert os.lseek(backing.fileno(), 0, os.SEEK_CUR) == 0
            assert mapped.size() == 32
            mapped.resize(16)
            assert len(mapped) == mapped.size() == 16
            assert mapped[:] == b"x" * 16

    # A unique named anonymous map can grow. Once another view is opened the
    # same resize must fail without losing either the data or the cursor.
    tag = str(uuid.uuid4())
    with mmap.mmap(-1, 8, tagname=tag) as mapped:
        mapped[:] = b"abcdefgh"
        mapped.resize(16)
        assert mapped[:8] == b"abcdefgh"
        with mmap.mmap(-1, 16, tagname=tag) as other:
            with assert_raises(OSError):
                mapped.resize(32)
            assert len(mapped) == 16
            assert mapped[:8] == other[:8] == b"abcdefgh"
        mapped.resize(32)
        assert mapped[:8] == b"abcdefgh"

    # Unnamed anonymous mappings must validate the offset too.
    with assert_raises(OSError):
        mmap.mmap(-1, 8, offset=1)
    with mmap.mmap(-1, 8, offset=mmap.ALLOCATIONGRANULARITY) as mapped:
        assert len(mapped) == 8
        mapped[:] = b"abcdefgh"
        mapped.resize(16)
        assert mapped[:8] == b"abcdefgh"

    # A failed resize can invalidate the map while retaining the backing file.
    # close() must release that handle even though closed already reports True.
    with tempfile.TemporaryDirectory() as directory:
        path = os.path.join(directory, "mapping")
        with open(path, "w+b") as backing:
            backing.write(b"abcdefgh")
            backing.flush()
            mapped = mmap.mmap(backing.fileno(), 8)
            with assert_raises(OSError):
                mapped.resize(0)
            assert mapped.closed
            mapped.close()
        os.unlink(path)
