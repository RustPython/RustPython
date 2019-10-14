from testutils import assert_raises


r = []

def make_numbers():
    yield 1
    yield 2
    r.append(42)
    yield 3

for a in make_numbers():
    r.append(a)

assert r == [1, 2, 42, 3]

r = list(x for x in [1, 2, 3])
assert r == [1, 2, 3]

def g2(x):
    x = yield x
    yield x + 5
    yield x + 7

i = g2(23)
assert 23 == next(i)
assert 15 == i.send(10)
assert 17 == i.send(10)


def g3():
    yield 23
    yield from make_numbers()
    yield 44
    yield from make_numbers()

r = list(g3())
# print(r)
assert r == [23, 1, 2, 3, 44, 1, 2, 3]

def g4():
    yield
    yield 2,

r = list(g4())
assert r == [None, (2,)]


def catch_exception():
    try:
        yield 1
    except ValueError:
        yield 2
        yield 3


g = catch_exception()
assert next(g) == 1

assert g.throw(ValueError, ValueError(), None) == 2
assert next(g) == 3

g = catch_exception()
assert next(g) == 1

with assert_raises(KeyError):
    assert g.throw(KeyError, KeyError(), None) == 2


r = []
def p(a, b, c):
    # print(a, b, c)
    r.append(a)
    r.append(b)
    r.append(c)


def g5():
    p('a', (yield 2), (yield 5))
    yield 99

g = g5()
g.send(None)
g.send(66)
# g.send(88)
l = list(g)
# print(r)
# print(l)
assert l == [99]
assert r == ['a', 66, None]

def binary(n):
    if n <= 1:
        return 1
    l = yield from binary(n - 1)
    r = yield from binary(n - 1)
    return l + 1 + r

with assert_raises(StopIteration):
    try:
        next(binary(5))
    except StopIteration as stopiter:
        # TODO: StopIteration.value
        assert stopiter.args[0] == 31
        raise

