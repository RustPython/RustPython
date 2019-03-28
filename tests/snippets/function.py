def foo():
    """test"""
    return 42

assert foo() == 42
assert foo.__doc__ == "test"


def my_func(a,):
    return a+2

assert my_func(2) == 4
