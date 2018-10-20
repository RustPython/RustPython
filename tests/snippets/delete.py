
a = 1
del a

class MyObject: pass
foo = MyObject()
foo.bar = 2
assert hasattr(foo, 'bar')
del foo.bar

assert not hasattr(foo, 'bar')

