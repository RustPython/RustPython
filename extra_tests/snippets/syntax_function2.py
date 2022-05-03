from testutils import assert_raises


__name__ = "function"


def foo():
    """test"""
    return 42

assert foo() == 42
assert foo.__doc__ == "test"
assert foo.__name__ == "foo"
assert foo.__qualname__ == "foo"
assert foo.__module__ == "function"
assert foo.__globals__ is globals()

def my_func(a,):
    return a+2

assert my_func(2) == 4

def fubar():
    return 42,

assert fubar() == (42,)

def f1():

    """test1"""
    pass

assert f1.__doc__ == "test1"

def f2():
    '''test2'''
    pass

assert f2.__doc__ == "test2"

def f3():
    """
    test3
    """
    pass

assert f3.__doc__ == "\n    test3\n    "

def f4():
    "test4"
    pass

assert f4.__doc__ == "test4"


def revdocstr(f):
    d = f.__doc__
    d = d + 'w00t'
    f.__doc__ = d
    return f

@revdocstr
def f5():
    """abc"""

assert f5.__doc__ == 'abcw00t', f5.__doc__


def f6():
    def nested():
        pass

    assert nested.__name__ == "nested"
    assert nested.__qualname__ == "f6.<locals>.nested"


f6()


def f7():
    try:
        def t() -> void: # noqa: F821
            pass
    except NameError:
        return True
    return False

assert f7()


def f8() -> int:
    return 10

assert f8() == 10


with assert_raises(SyntaxError):
    exec('print(keyword=10, 20)')

def f9():
    pass

assert f9.__doc__ == None
