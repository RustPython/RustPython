from testutils import assert_raises


def sum(x, y):
    return x+y

def total(a, b, c, d):
    return sum(sum(a,b), sum(c,d))


assert total(1,1,1,1) == 4
assert total(1,2,3,4) == 10

assert sum(1, 1) == 2
assert sum(1, 3) == 4


def sum2y(x, y):
    return x+y*2


assert sum2y(1, 1) == 3
assert sum2y(1, 3) == 7


def va(a, b=2, *c, d, **e):
    assert a == 1
    assert b == 22
    assert c == (3, 4)
    assert d == 1337
    assert e['f'] == 42


va(1, 22, 3, 4, d=1337, f=42)

assert va.__defaults__ == (2,)
assert va.__kwdefaults__ is None


def va2(*args, **kwargs):
    assert args == (5, 4)
    assert len(kwargs) == 0

va2(5, 4)
x = (5, 4)
va2(*x)

va2(5, *x[1:])


def va3(x, *, a, b=2, c=9):
    return x + b + c


assert va3(1, a=1, b=10) == 20

with assert_raises(TypeError):
    va3(1, 2, 3, a=1, b=10)

with assert_raises(TypeError):
    va3(1, b=10)


assert va3.__defaults__ is None
kw_defaults = va3.__kwdefaults__
# assert va3.__kwdefaults__ == {'b': 2, 'c': 9}
assert set(kw_defaults) == {'b', 'c'}
assert kw_defaults['b'] == 2
assert kw_defaults['c'] == 9

x = {'f': 42, 'e': 1337}
y = {'d': 1337}
va(1, 22, 3, 4, **x, **y)

# star arg after keyword args:
def fubar(x, y, obj=None):
    assert x == 4
    assert y == 5
    assert obj == 6

rest = [4, 5]
fubar(obj=6, *rest)


# https://www.python.org/dev/peps/pep-0468/
def func(**kwargs):
    return list(kwargs.items())

empty_kwargs = func()
assert empty_kwargs == []

kwargs = func(a=1, b=2)
assert kwargs == [('a', 1), ('b', 2)]

kwargs = func(a=1, b=2, c=3)
assert kwargs == [('a', 1), ('b', 2), ('c', 3)]


def inc(n):
    return n + 1

with assert_raises(SyntaxError):
    exec("inc(n=1, n=2)")

with assert_raises(SyntaxError):
    exec("def f(a=1, b): pass")


def f(a):
    pass

x = {'a': 1}
y = {'a': 2}
with assert_raises(TypeError):
    f(**x, **y)


def f(a, b, /, c, d, *, e, f):
    return a + b + c + d + e + f

assert f(1,2,3,4,e=5,f=6) == 21
assert f(1,2,3,d=4,e=5,f=6) == 21
assert f(1,2,c=3,d=4,e=5,f=6) == 21
with assert_raises(TypeError):
    f(1,b=2,c=3,d=4,e=5,f=6)
with assert_raises(TypeError):
    f(a=1,b=2,c=3,d=4,e=5,f=6)
with assert_raises(TypeError):
    f(1,2,3,4,5,f=6)
with assert_raises(TypeError):
    f(1,2,3,4,5,6)
