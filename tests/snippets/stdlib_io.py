from io import BufferedReader, FileIO, StringIO, BytesIO
import os
from testutils import assert_raises

fi = FileIO('README.md')
assert fi.seekable()
bb = BufferedReader(fi)
assert bb.seekable()

result = bb.read()

assert len(result) <= 8*1024
assert len(result) >= 0
assert isinstance(result, bytes)

with FileIO('README.md') as fio:
	res = fio.read()
	assert len(result) <= 8*1024
	assert len(result) >= 0
	assert isinstance(result, bytes)

fd = os.open('README.md', os.O_RDONLY)

with FileIO(fd) as fio:
	res2 = fio.read()
	assert res == res2

fi = FileIO('README.md')
fi.read()
fi.close()
assert fi.closefd
assert fi.closed

with assert_raises(ValueError):
    fi.read()

with FileIO('README.md') as fio:
	nres = fio.read(1)
	assert len(nres) == 1
	nres = fio.read(2)
	assert len(nres) == 2
