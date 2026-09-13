import sys
from collections import deque
from typing import Deque

from testutils import assert_raises


def test_deque_iterator__new__():
    klass = type(iter(deque()))
    s = "abcd"
    d = klass(deque(s))
    assert list(d) == list(s)


test_deque_iterator__new__()


def test_deque_iterator__new__positional_index():
    klass = type(iter(deque()))

    # index between 0 and len
    for s in ("abcd", range(200)):
        for i in range(len(s)):
            d = klass(deque(s), i)
            assert list(d) == list(s)[i:]

    # negative index
    for s in ("abcd", range(200)):
        for i in range(-100, 0):
            d = klass(deque(s), i)
            assert list(d) == list(s)

    # index ge len
    for s in ("abcd", range(200)):
        for i in range(len(s), 400):
            d = klass(deque(s), i)
            assert list(d) == list()


test_deque_iterator__new__positional_index()


def test_deque_iterator__new__not_using_keyword_index():
    klass = type(iter(deque()))

    for s in ("abcd", range(200)):
        for i in range(-100, 400):
            d = klass(deque(s), index=i)
            assert list(d) == list(s)


test_deque_iterator__new__not_using_keyword_index()


def test_deque_reverse_iterator__new__positional_index():
    klass = type(reversed(deque()))

    # index between 0 and len
    for s in ("abcd", range(200)):
        for i in range(len(s)):
            d = klass(deque(s), i)
            assert list(d) == list(reversed(s))[i:]

    # negative index
    for s in ("abcd", range(200)):
        for i in range(-100, 0):
            d = klass(deque(s), i)
            assert list(d) == list(reversed(s))

    # index ge len
    for s in ("abcd", range(200)):
        for i in range(len(s), 400):
            d = klass(deque(s), i)
            assert list(d) == list()


test_deque_reverse_iterator__new__positional_index()


def test_deque_reverse_iterator__new__not_using_keyword_index():
    klass = type(reversed(deque()))

    for s in ("abcd", range(200)):
        for i in range(-100, 400):
            d = klass(deque(s), index=i)
            assert list(d) == list(reversed(s))


test_deque_reverse_iterator__new__not_using_keyword_index()

assert repr(deque()) == "deque([])"
assert repr(deque([1, 2, 3])) == "deque([1, 2, 3])"


class D(deque):
    pass


assert repr(D()) == "D([])"
assert repr(D([1, 2, 3])) == "D([1, 2, 3])"


assert_raises(ValueError, lambda: deque().index(10, 0, 10000000000000000000000000))

if sys.implementation.name == "rustpython":
    # The repeat count is multiplied by the length; a count that overflows that
    # product must be rejected up front. CPython instead appends block by block
    # until the allocator gives up, so it is left out of this check.
    with assert_raises(MemoryError):
        deque([0]) * sys.maxsize


# maxlen=0 keeps nothing, whichever end the item arrives at.
d = deque(maxlen=0)
d.append(1)
d.appendleft(2)
assert list(d) == []
assert len(d) == 0
assert d.maxlen == 0

d = deque(maxlen=0)
d.extend("abc")
d.extendleft("abc")
d += "abc"
assert list(d) == []

assert list(deque("abc", maxlen=0)) == []
assert list(deque("ab", maxlen=0) * 3) == []
assert list(deque("ab", maxlen=0) + deque("cd")) == []

d = deque("abc", maxlen=0)
d.rotate(1)
assert list(d) == []

assert_raises(IndexError, deque(maxlen=0).insert, 0, 1)


# A bounded deque still drops from the far end, and only once it is full.
d = deque(maxlen=1)
d.append(1)
assert list(d) == [1]
d.append(2)
assert list(d) == [2]
d.appendleft(3)
assert list(d) == [3]

d = deque("ab", maxlen=3)
d.append("c")
assert list(d) == ["a", "b", "c"]
d.append("d")
assert list(d) == ["b", "c", "d"]
d.appendleft("z")
assert list(d) == ["z", "b", "c"]
