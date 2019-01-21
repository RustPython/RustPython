from io import BufferedReader, FileIO

fi = FileIO('Cargo.toml')
bb = BufferedReader(fi)

result = bb.read()

assert len(result) <= 8*1024
assert len(result) >= 0

