from io import BufferedReader, FileIO

fi = FileIO('README.md')
bb = BufferedReader(fi)

result = bb.read()

assert len(result) <= 8*1024
assert len(result) >= 0
assert isinstance(result, bytes)
