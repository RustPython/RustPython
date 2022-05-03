from testutils import assert_raises


class Fubar:
    def __init__(self):
        self.x = 100

    @property
    def foo(self):
        value = self.x
        self.x += 1
        return value


f = Fubar()
assert f.foo == 100
assert f.foo == 101

assert type(Fubar.foo) is property


class Bar:
    def __init__(self):
        self.a = 0

    @property
    def foo(self):
        return self.a

    @foo.setter
    def foo(self, value):
        self.a += value

    @foo.deleter
    def foo(self):
        self.a -= 1


bar = Bar()
assert bar.foo == 0
bar.foo = 5
assert bar.a == 5
del bar.foo
assert bar.a == 4
del bar.foo
assert bar.a == 3


null_property = property()
assert type(null_property) is property

p = property(lambda x: x[0])
assert p.__get__((2,), tuple) == 2
assert p.__get__((2,)) == 2

with assert_raises(AttributeError):
    null_property.__get__((), tuple)

with assert_raises(TypeError):
    property.__new__(object)

assert p.__doc__ is None

# Test property instance __doc__ attribute:
p.__doc__ = '222'
assert p.__doc__ == '222'


p1 = property("a", "b", "c")

assert p1.fget == "a"
assert p1.fset == "b"
assert p1.fdel == "c"

assert p1.getter(1).fget == 1
assert p1.setter(2).fset == 2
assert p1.deleter(3).fdel == 3

assert p1.getter(None).fget == "a"
assert p1.setter(None).fset == "b"
assert p1.deleter(None).fdel == "c"

assert p1.__get__(None, object) is p1
# assert p1.__doc__ is 'a'.__doc__

p2 = property('a', doc='pdoc')
# assert p2.__doc__ == 'pdoc'
