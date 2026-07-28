import mmap


mapped = mmap.mmap(-1, 1)
assert mapped.seekable()
mapped.close()
assert mapped.seekable()
