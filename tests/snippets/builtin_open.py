fd = open('README.md')
assert 'RustPython' in fd.read()

try:
    open('DoesNotExist')
    assert False
except FileNotFoundError:
    pass
