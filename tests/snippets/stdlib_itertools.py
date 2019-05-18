import itertools

from testutils import assertRaises


# itertools.count tests

# default arguments
c = itertools.count()
assert next(c) == 0
assert next(c) == 1
assert next(c) == 2

# positional
c = itertools.count(2, 3)
assert next(c) == 2
assert next(c) == 5
assert next(c) == 8

# backwards
c = itertools.count(1, -10)
assert next(c) == 1
assert next(c) == -9
assert next(c) == -19

# step = 0
c = itertools.count(5, 0)
assert next(c) == 5
assert next(c) == 5

# itertools.count TODOs: kwargs and floats

# step kwarg
# c = itertools.count(step=5)
# assert next(c) == 0
# assert next(c) == 5

# start kwarg
# c = itertools.count(start=10)
# assert next(c) == 10

# float start
# c = itertools.count(0.5)
# assert next(c) == 0.5
# assert next(c) == 1.5
# assert next(c) == 2.5

# float step
# c = itertools.count(1, 0.5)
# assert next(c) == 1
# assert next(c) == 1.5
# assert next(c) == 2

# float start + step
# c = itertools.count(0.5, 0.5)
# assert next(c) == 0.5
# assert next(c) == 1
# assert next(c) == 1.5


# itertools.repeat tests

# no times
r = itertools.repeat(5)
assert next(r) == 5
assert next(r) == 5
assert next(r) == 5

# times
r = itertools.repeat(1, 2)
assert next(r) == 1
assert next(r) == 1
with assertRaises(StopIteration):
    next(r)

# timees = 0
r = itertools.repeat(1, 0)
with assertRaises(StopIteration):
    next(r)

# negative times
r = itertools.repeat(1, -1)
with assertRaises(StopIteration):
    next(r)


# itertools.starmap tests
starmap = itertools.starmap

assert list(starmap(pow, zip(range(3), range(1,7)))) ==  [0**1, 1**2, 2**3]
assert list(starmap(pow, [])) == []
assert list(starmap(pow, [iter([4,5])])) == [4**5]
with assertRaises(TypeError):
    starmap(pow)


# itertools.takewhile tests

from itertools import takewhile as tw

t = tw(lambda n: n < 5, [1, 2, 5, 1, 3])
assert next(t) == 1
assert next(t) == 2
with assertRaises(StopIteration):
    next(t)

# not iterable
with assertRaises(TypeError):
    tw(lambda n: n < 1, 1)

# not callable
t = tw(5, [1, 2])
with assertRaises(TypeError):
    next(t)

# non-bool predicate
t = tw(lambda n: n, [1, 2, 0])
assert next(t) == 1
assert next(t) == 2
with assertRaises(StopIteration):
    next(t)

# bad predicate prototype
t = tw(lambda: True, [1])
with assertRaises(TypeError):
    next(t)

# StopIteration before attempting to call (bad) predicate
t = tw(lambda: True, [])
with assertRaises(StopIteration):
    next(t)

# doesn't try again after the first predicate failure
t = tw(lambda n: n < 1, [1, 0])
with assertRaises(StopIteration):
    next(t)
with assertRaises(StopIteration):
    next(t)
