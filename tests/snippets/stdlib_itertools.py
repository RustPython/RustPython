import itertools

from testutils import assert_raises


# itertools.chain tests
chain = itertools.chain

# empty
assert list(chain()) == []
assert list(chain([], "", b"", ())) == []

assert list(chain([1, 2, 3, 4])) == [1, 2, 3, 4]
assert list(chain("ab", "cd", (), 'e')) == ['a', 'b', 'c', 'd', 'e']
with assert_raises(TypeError):
    list(chain(1))

x = chain("ab", 1)
assert next(x) == 'a'
assert next(x) == 'b'
with assert_raises(TypeError):
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
with assert_raises(StopIteration):
    next(r)

# timees = 0
r = itertools.repeat(1, 0)
with assert_raises(StopIteration):
    next(r)

# negative times
r = itertools.repeat(1, -1)
with assert_raises(StopIteration):
    next(r)


# itertools.starmap tests
starmap = itertools.starmap

assert list(starmap(pow, zip(range(3), range(1,7)))) ==  [0**1, 1**2, 2**3]
assert list(starmap(pow, [])) == []
assert list(starmap(pow, [iter([4,5])])) == [4**5]
with assert_raises(TypeError):
    starmap(pow)


# itertools.takewhile tests

from itertools import takewhile as tw

t = tw(lambda n: n < 5, [1, 2, 5, 1, 3])
assert next(t) == 1
assert next(t) == 2
with assert_raises(StopIteration):
    next(t)

# not iterable
with assert_raises(TypeError):
    tw(lambda n: n < 1, 1)

# not callable
t = tw(5, [1, 2])
with assert_raises(TypeError):
    next(t)

# non-bool predicate
t = tw(lambda n: n, [1, 2, 0])
assert next(t) == 1
assert next(t) == 2
with assert_raises(StopIteration):
    next(t)

# bad predicate prototype
t = tw(lambda: True, [1])
with assert_raises(TypeError):
    next(t)

# StopIteration before attempting to call (bad) predicate
t = tw(lambda: True, [])
with assert_raises(StopIteration):
    next(t)

# doesn't try again after the first predicate failure
t = tw(lambda n: n < 1, [1, 0])
with assert_raises(StopIteration):
    next(t)
with assert_raises(StopIteration):
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
with assert_raises(StopIteration):
    next(it)

l = [0, 1, None, False, True, [], {}]
it = itertools.filterfalse(None, l)
assert 0 == next(it)
assert None == next(it)
assert False == next(it)
assert [] == next(it)
assert {} == next(it)


# itertools.dropwhile
it = itertools.dropwhile(lambda x: x<5, [1,4,6,4,1])
assert 6 == next(it)
assert 4 == next(it)
assert 1 == next(it)
with assert_raises(StopIteration):
    next(it)


# itertools.accumulate
it = itertools.accumulate([6, 3, 7, 1, 0, 9, 8, 8])
assert 6 == next(it)
assert 9 == next(it)
assert 16 == next(it)
assert 17 == next(it)
assert 17 == next(it)
assert 26 == next(it)
assert 34 == next(it)
assert 42 == next(it)
with assert_raises(StopIteration):
    next(it)

it = itertools.accumulate([3, 2, 4, 1, 0, 5, 8], lambda a, v: a*v)
assert 3 == next(it)
assert 6 == next(it)
assert 24 == next(it)
assert 24 == next(it)
assert 0 == next(it)
assert 0 == next(it)
assert 0 == next(it)
with assert_raises(StopIteration):
    next(it)

# itertools.compress
assert list(itertools.compress("ABCDEF", [1,0,1,0,1,1])) == list("ACEF")
assert list(itertools.compress("ABCDEF", [0,0,0,0,0,0])) == list("")
assert list(itertools.compress("ABCDEF", [1,1,1,1,1,1])) == list("ABCDEF")
assert list(itertools.compress("ABCDEF", [1,0,1])) == list("AC")
assert list(itertools.compress("ABC", [0,1,1,1,1,1])) == list("BC")
assert list(itertools.compress("ABCDEF", [True,False,"t","",1,9])) == list("ACEF")
