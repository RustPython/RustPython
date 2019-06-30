from io import BufferedReader, FileIO
import os

fi = FileIO('README.md')
bb = BufferedReader(fi)

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
