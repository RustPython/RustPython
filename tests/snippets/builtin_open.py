from testutils import assert_raises

fd = open('README.md')
assert 'RustPython' in fd.read()

assert_raises(FileNotFoundError, open, 'DoesNotExist')

# Use open as a context manager
with open('README.md', 'rt') as fp:
    contents = fp.read()
    assert type(contents) == str, "type is " + str(type(contents))

with open('README.md', 'r') as fp:
    contents = fp.read()
    assert type(contents) == str, "type is " + str(type(contents))

with open('README.md', 'rb') as fp:
    contents = fp.read()
    assert type(contents) == bytes, "type is " + str(type(contents))
