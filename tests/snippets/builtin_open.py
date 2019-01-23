c1 = open('README.md').read()

assert isinstance(c1, str)
assert 0 < len(c1)

c2 = open('README.md', 'rb').read()

assert isinstance(c2, bytes)
assert 0 < len(c2)
