from testutils import assert_raises
import io

print(2 + 3)

assert_raises(TypeError, print, 'test', end=4, _msg='wrong type passed to end')
assert_raises(TypeError, print, 'test', sep=['a'], _msg='wrong type passed to sep')

try:
    print('test', end=None, sep=None, flush=None)
except:
    assert False, 'Expected None passed to end, sep, and flush to not raise errors'

buf = io.StringIO()
print('hello, world', file=buf)
assert buf.getvalue() == 'hello, world\n', buf.getvalue()
