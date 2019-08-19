import itertools

from testutils import assertRaises


# itertools.chain tests
chain = itertools.chain

# empty
assert list(chain()) == []
assert list(chain([], "", b"", ())) == []

assert list(chain([1, 2, 3, 4])) == [1, 2, 3, 4]
assert list(chain("ab", "cd", (), 'e')) == ['a', 'b', 'c', 'd', 'e']
with assertRaises(TypeError):
    list(chain(1))

x = chain("ab", 1)
assert next(x) == 'a'
assert next(x) == 'b'
with assertRaises(TypeError):
    next(x)

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


# itertools.islice tests

def assert_matches_seq(it, seq):
    assert list(it) == list(seq)

i = itertools.islice

it = i([1, 2, 3, 4, 5], 3)
assert_matches_seq(it, [1, 2, 3])

it = i([0.5, 1, 1.5, 2, 2.5, 3, 4, 5], 1, 6, 2)
assert_matches_seq(it, [1, 2, 3])

it = i([1, 2], None)
assert_matches_seq(it, [1, 2])

it = i([1, 2, 3], None, None, None)
assert_matches_seq(it, [1, 2, 3])

it = i([1, 2, 3], 1, None, None)
assert_matches_seq(it, [2, 3])

it = i([1, 2, 3], None, 2, None)
assert_matches_seq(it, [1, 2])

it = i([1, 2, 3], None, None, 3)
assert_matches_seq(it, [1])

# itertools.filterfalse
it = itertools.filterfalse(lambda x: x%2, range(10))
assert 0 == next(it)
assert 2 == next(it)
assert 4 == next(it)
assert 6 == next(it)
assert 8 == next(it)
with assertRaises(StopIteration):
    next(it)

l = [0, 1, None, False, True, [], {}]
it = itertools.filterfalse(None, l)
assert 0 == next(it)
assert None == next(it)
assert False == next(it)
assert [] == next(it)
assert {} == next(it)