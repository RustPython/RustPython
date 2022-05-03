
def logged(f):
    def wrapper(a, b):
        print('Calling function', f)
        return f(a, b + 1)
    return wrapper


@logged
def add(a, b):
    return a + b

c = add(10, 3)

assert c == 14


@logged
def add3(a, b, c=2):
    return a + b + c


d = add3(12, 5)

assert d == 20


def f(func): return lambda: 42
class A: pass
a = A()
a.a = A()
a.a.x = f

@a.a.x
def func():
	pass

assert func() == 42
