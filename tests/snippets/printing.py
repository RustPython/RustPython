from testutils import assert_raises

print(2 + 3)

assert_raises(TypeError, lambda: print('test', end=4), 'wrong type passed to end')
assert_raises(TypeError, lambda: print('test', sep=['a']), 'wrong type passed to sep')

try:
    print('test', end=None, sep=None, flush=None)
except:
    assert False, 'Expected None passed to end, sep, and flush to not raise errors'
