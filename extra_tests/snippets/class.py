__name__ = "class"


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
assert Foo.__name__ == "Foo"
assert Foo.__qualname__ == "Foo"
assert Foo.__module__ == "class"
assert Foo.square.__name__ == "square"
assert Foo.square.__qualname__ == "Foo.square"
assert Foo.square.__module__ == "class"

class Bar:
    """ W00t """
    def __init__(self, x):
        self.x = x

    def get_x(self):
        assert __class__ is Bar
        return self.x

    def doc_func(self):
        "doc string"
        pass

    @classmethod
    def fubar(cls, x):
        assert __class__ is cls
        assert cls is Bar
        assert x == 2

    @staticmethod
    def kungfu(x):
        assert __class__ is Bar
        assert x == 3


assert Bar.__doc__ == " W00t "

bar = Bar(42)
assert bar.get_x.__doc__ == None
assert bar.doc_func.__doc__ == "doc string"

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

    @classmethod
    def test3(cls):
        return 300

class C(A,B):
    def test(self):
        return super().test()

    def test1(self):
        return super().test1()

    @classmethod
    def test3(cls):
        return super().test3()

c = C()
assert c.test() == 100
assert c.test1() == 200
assert c.test3() == 300
assert C.test3() == 300

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


class T1:
    "test1"

assert T1.__doc__ == "test1"

class T2:
    '''test2'''

assert T2.__doc__ == "test2"

class T3:
    """
    test3
    """

assert T3.__doc__ == "\n    test3\n    "

class T4:

    """test4"""

    def t1(self):
        """t1"""
        pass

assert T4.__doc__ == "test4"
assert T4.t1.__doc__ == "t1"

cm = classmethod(lambda cls: cls)
assert cm.__func__(int) is int

assert str(super(int, 5)) == "<super: <class 'int'>, <int object>>"

class T5(int):
    pass

assert str(super(int, T5(5))) == "<super: <class 'int'>, <T5 object>>"

assert str(super(type, None)) == "<super: <class 'type'>, NULL>"

assert str(super(int).__get__(T5(5))) == "<super: <class 'int'>, <T5 object>>"

a = 1
class A:
    a = 2
    def b():
        assert a == 1
    b()
    assert a == 2
A.b()

class A:
    pass

assert A.__doc__ == None

class B:
    "Docstring"

assert B.__doc__ == "Docstring"

# TODO: uncomment once free vars/cells are working
# The symboltable sees that b() is referring to a in the nested scope,
# so it marks it as non local. When it's executed, it walks up the scopes
# and still finds the a from the class scope.
# a = 1
# def nested_scope():
#     a = 2
#     class A:
#         a = 3
#         def b():
#             assert a == 2
#         b()
#         assert a == 3
#     A.b()
# nested_scope()



# Multiple inheritance and mro tests.
class A():
    def f(self):
        return 'a'

class B(A):
    def f(self):
        return 'b' + super().f()

class C(A):
    def f(self):
        return 'c' + super().f()

class D(B, C):
    def f(self):
        return 'd' + super().f()

assert D().f() == 'dbca', "Mro resolution using super failed."



class A():
    pass
try:
    class B(A, A):
        pass
except TypeError:
    pass
else:
    assert False, "Managed to create a class with duplicate base classes."


class A():
    pass
class B(A):
    pass
try:
    class C(A, B):
        pass
except TypeError:
    pass
else:
    assert False, "Managed to create a class without local type precedence."


class A():
    a: int

assert A.__annotations__['a'] == int

class A: pass
class B: pass

class C(*(A, B), **{"metaclass": type}):
    pass
