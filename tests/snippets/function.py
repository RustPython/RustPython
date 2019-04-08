def foo():
    """test"""
    return 42

assert foo() == 42
assert foo.__doc__ == "test"

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

