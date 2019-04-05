def sum(x, y):
    return x+y

# def total(a, b, c, d):
#     return sum(sum(a,b), sum(c,d))
#
# assert total(1,1,1,1) == 4
# assert total(1,2,3,4) == 10

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

def va2(*args, **kwargs):
    assert args == (5, 4)
    assert len(kwargs) == 0

va2(5, 4)
x = (5, 4)
va2(*x)

va2(5, *x[1:])
# def va3(x, *, b=2):
#    pass

# va3(1, 2, 3, b=10)

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
