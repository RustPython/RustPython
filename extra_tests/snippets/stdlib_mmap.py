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
