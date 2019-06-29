from testutils import assert_raises, assertRaises

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
assert_raises(NameError, lambda: x)
assert_raises(NameError, lambda: y)

with assertRaises(NameError):
    del y
