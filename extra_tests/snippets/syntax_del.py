from testutils import assert_raises

a = 1
del a

class MyObject: pass
foo = MyObject()
foo.bar = 2
assert hasattr(foo, 'bar')
del foo.bar

assert not hasattr(foo, 'bar')

x = 1
y = 2
del (x, y)
assert_raises(NameError, lambda: x)  # noqa: F821
assert_raises(NameError, lambda: y)  # noqa: F821

with assert_raises(NameError):
    del y  # noqa: F821
