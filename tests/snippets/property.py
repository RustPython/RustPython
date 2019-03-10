from testutils import assertRaises


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


null_property = property()
assert type(null_property) is property

p = property(lambda x: x[0])
assert p.__get__((2,), tuple) == 2
# TODO owner parameter is optional
# assert p.__get__((2,)) == 2

with assertRaises(AttributeError):
    null_property.__get__((), tuple)

with assertRaises(TypeError):
    property.__new__(object)
