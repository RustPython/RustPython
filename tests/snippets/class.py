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


class Bar:
    """ W00t """
    def __init__(self, x):
        self.x = x

    def get_x(self):
        assert __class__ is Bar
        return self.x

    @classmethod
    def fubar(cls, x):
        assert __class__ is cls
        assert cls is Bar
        assert x == 2

    @staticmethod
    def kungfu(x):
        assert __class__ is Bar
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

bar2 = Bar2()
assert bar2.get_x() == 101

class A():
    def test(self):
        return 100

class B():
    def test1(self):
        return 200

class C(A,B):
    def test(self):
        return super().test()

    def test1(self):
        return super().test1()

c = C()
assert c.test() == 100
assert c.test1() == 200

class Me():

    def test(me):
        return 100

class Me2(Me):

    def test(me):
        return super().test()

class A():
    def f(self):
        pass

class B(A):
    def f(self):
        super().f()

class C(B):
    def f(self):
        super().f()

C().f()

me = Me2()
assert me.test() == 100

a = super(bool, True)
assert isinstance(a, super)
assert type(a) is super
assert a.conjugate() == 1

