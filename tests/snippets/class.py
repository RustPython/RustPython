class Foo:
    def __init__(self, x):
        assert x == 5
        self.x = x

    def square(self):
        return self.x * self.x

    y = 7


foo = Foo(5)

assert foo.y == Foo.y
assert foo.x == 5
assert foo.square() == 25


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


class Bar:
    """ W00t """
    def __init__(self, x):
        self.x = x

    def get_x(self):
        return self.x

    @classmethod
    def fubar(cls, x):
        assert cls is Bar
        assert x == 2

    @staticmethod
    def kungfu(x):
        assert x == 3


# TODO:
# assert Bar.__doc__ == " W00t "

bar = Bar(42)

bar.fubar(2)
Bar.fubar(2)

bar.kungfu(3)
Bar.kungfu(3)


class Bar2(Bar):
    def __init__(self):
        super().__init__(101)


# TODO: make this work:
# bar2 = Bar2()
# assert bar2.get_x() == 101

a = super(int, 2)
assert isinstance(a, super)
assert type(a) is super

