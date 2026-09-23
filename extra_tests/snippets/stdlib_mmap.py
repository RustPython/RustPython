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
# offsets past the end are clamped and negative ones count from the end
assert mapped.find(b"", 5) == 3
assert mapped.rfind(b"", 5) == 3
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
