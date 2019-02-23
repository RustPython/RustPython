from testutils import assert_raises

fd = open('README.md')
assert 'RustPython' in fd.read()

assert_raises(FileNotFoundError, lambda: open('DoesNotExist'))
