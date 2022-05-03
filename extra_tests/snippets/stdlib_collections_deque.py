from collections import deque
from typing import Deque


def test_deque_iterator__new__():
    klass = type(iter(deque()))
    s = 'abcd'
    d = klass(deque(s))
    assert (list(d) == list(s))


test_deque_iterator__new__()


def test_deque_iterator__new__positional_index():
    klass = type(iter(deque()))

    # index between 0 and len
    for s in ('abcd', range(2000)):
        for i in range(len(s)):
            d = klass(deque(s), i)
            assert (list(d) == list(s)[i:])

    # negative index
    for s in ('abcd', range(2000)):
        for i in range(-1000, 0):
            d = klass(deque(s), i)
            assert (list(d) == list(s))

    # index ge len
    for s in ('abcd', range(2000)):
        for i in range(len(s), 4000):
            d = klass(deque(s), i)
            assert (list(d) == list())


test_deque_iterator__new__positional_index()


def test_deque_iterator__new__not_using_keyword_index():
    klass = type(iter(deque()))

    for s in ('abcd', range(2000)):
        for i in range(-1000, 4000):
            d = klass(deque(s), index=i)
            assert (list(d) == list(s))


test_deque_iterator__new__not_using_keyword_index()


def test_deque_reverse_iterator__new__positional_index():
    klass = type(reversed(deque()))

    # index between 0 and len
    for s in ('abcd', range(2000)):
        for i in range(len(s)):
            d = klass(deque(s), i)
            assert (list(d) == list(reversed(s))[i:])

    # negative index
    for s in ('abcd', range(2000)):
        for i in range(-1000, 0):
            d = klass(deque(s), i)
            assert (list(d) == list(reversed(s)))

    # index ge len
    for s in ('abcd', range(2000)):
        for i in range(len(s), 4000):
            d = klass(deque(s), i)
            assert (list(d) == list())


test_deque_reverse_iterator__new__positional_index()


def test_deque_reverse_iterator__new__not_using_keyword_index():
    klass = type(reversed(deque()))

    for s in ('abcd', range(2000)):
        for i in range(-1000, 4000):
            d = klass(deque(s), index=i)
            assert (list(d) == list(reversed(s)))


test_deque_reverse_iterator__new__not_using_keyword_index()

assert repr(deque()) == "deque([])"
assert repr(deque([1, 2, 3])) == "deque([1, 2, 3])"

class D(deque):
    pass

assert repr(D()) == "D([])"
assert repr(D([1, 2, 3])) == "D([1, 2, 3])"
