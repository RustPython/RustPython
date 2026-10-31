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
