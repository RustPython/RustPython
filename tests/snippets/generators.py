from testutils import assertRaises


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

r = list(g3())
# print(r)
assert r == [23, 1, 2, 3, 44]

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

with assertRaises(KeyError):
    assert g.throw(KeyError, KeyError(), None) == 2
