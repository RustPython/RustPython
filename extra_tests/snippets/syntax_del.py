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

# see https://github.com/RustPython/RustPython/issues/4863

class MyTest:
    def __del__(self):
        type(self)()

def test_del_panic():
    mytest = MyTest()
    del mytest

# see https://github.com/RustPython/RustPython/issues/4910

def f():
    del b # noqa

b = 'a'
assert_raises(UnboundLocalError, f)
