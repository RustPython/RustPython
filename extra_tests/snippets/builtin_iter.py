import queue
import threading


def make_iterator():
    holder = {}

    class Evil:
        def __getitem__(self, index):
            if index == 0:
                return 0
            raise IndexError

        def __len__(self):
            return holder["it"].__length_hint__()

    obj = Evil()
    holder["it"] = iter(obj)
    return holder["it"]


it = make_iterator()
q = queue.Queue()


def run():
    try:
        it.__length_hint__()
    except Exception as exc:  # noqa: BLE001
        q.put(exc)
    else:
        q.put(None)


t = threading.Thread(target=run, daemon=True)
t.start()
t.join(1)

assert not t.is_alive(), "iterator.__length_hint__ deadlocked"
err = q.get_nowait()
assert isinstance(err, RecursionError)


class NoLen:
    def __getitem__(self, index):
        if index < 3:
            return index
        raise IndexError


no_len_it = iter(NoLen())
assert no_len_it.__length_hint__() is NotImplemented
next(no_len_it)
assert no_len_it.__length_hint__() is NotImplemented


class Seq:
    def __init__(self):
        self.items = [1, 2, 3]

    def __getitem__(self, index):
        return self.items[index]

    def __len__(self):
        return len(self.items)


seq_it = iter(Seq())
assert seq_it.__length_hint__() == 3
next(seq_it)
assert seq_it.__length_hint__() == 2


# Walking an iterator takes no room up front, so nothing on the way asks it how
# long it is. Only join does, reaching its elements through PySequence_Fast(),
# which fills a list from the iterator.
import array
import collections
import io
import math


class LoudIterator:
    def __init__(self, seq):
        self.i = iter(seq)

    def __iter__(self):
        return self

    def __next__(self):
        return next(self.i)

    def __length_hint__(self):
        raise NotImplementedError("iterator hint")


def handing(seq=(1, 2, 3)):
    class Handing:
        def __iter__(self):
            return LoudIterator(seq)

    return Handing()


assert set(handing()) == {1, 2, 3}
assert frozenset(handing()) == frozenset({1, 2, 3})
assert {1}.difference(handing()) == set()
assert {1}.intersection(handing()) == {1}
assert {1}.symmetric_difference(handing()) == {2, 3}
assert {1}.issubset(handing())
assert not {9}.issuperset(handing())
assert dict.fromkeys(handing()) == {1: None, 2: None, 3: None}
assert array.array("b", handing()) == array.array("b", [1, 2, 3])
assert all(handing()) and any(handing())
assert sum(handing()) == 6
assert math.fsum(handing()) == 6.0
assert math.prod(handing()) == 6
assert collections.deque(handing()) == collections.deque([1, 2, 3])
assert tuple(handing()) == (1, 2, 3)
assert list(handing()) == [1, 2, 3]
assert min(handing()) == 1
assert bytes(handing()) == b"\x01\x02\x03"
assert bytearray(handing()) == bytearray(b"\x01\x02\x03")
io.StringIO().writelines(handing(("a", "b")))

# join asks, and answers with what asking raised.
for empty in ("", b""):
    try:
        empty.join(handing((empty.__class__(),)))
    except NotImplementedError:
        pass
    else:
        raise AssertionError(f"{empty.__class__.__name__}.join did not ask")


# An error from an element is the element's, not the end of the walk, so the
# next step reaches for the same one again.
class Balky:
    def __getitem__(self, i):
        if i == 1:
            raise ValueError("boom")
        if i > 2:
            raise IndexError
        return i


it = iter(Balky())
assert next(it) == 0
for _ in range(2):
    try:
        next(it)
    except ValueError as e:
        assert str(e) == "boom", e
    else:
        raise AssertionError("the element's error did not reach the caller")


# A collection that moved under its iterator raises every time it is asked
# again, rather than reading as spent after the first. What is left to walk
# reads as nothing from the moment the collection no longer matches.
from collections import deque
from operator import length_hint


def moved(make, mutate, restore, moved_hint, again):
    it = make()
    next(it)
    assert length_hint(it) == 9, length_hint(it)
    mutate()
    assert length_hint(it) == moved_hint, length_hint(it)
    try:
        next(it)
    except RuntimeError:
        pass
    else:
        raise AssertionError("a collection that moved was walked further")
    assert length_hint(it) == 0, length_hint(it)
    restore()
    try:
        next(it)
    except RuntimeError:
        got = RuntimeError
    except StopIteration:
        got = StopIteration
    else:
        raise AssertionError("a collection that moved was walked further")
    assert got is again, got
    assert length_hint(it) == 0, length_hint(it)


# A deque iterator carries its own count, so what the deque does to its own
# length before the iterator is asked again is not what the count answers.
d = deque(range(10))
moved(lambda: iter(d), d.pop, lambda: d.append(99), 9, RuntimeError)
d2 = deque(range(10))
# `dequereviter_next()` looks at the count before the deque, so once the count
# is spent the deque is never looked at again.
moved(lambda: reversed(d2), d2.pop, lambda: d2.append(99), 9, StopIteration)

# A dict or set iterator answers from the size it captured, which the
# collection stops matching the moment it changes.
s = set(range(10))
moved(lambda: iter(s), lambda: s.add(99), lambda: s.discard(99), 0, RuntimeError)
dd = {i: i for i in range(10)}
moved(
    lambda: iter(dd), lambda: dd.update({99: 99}), lambda: dd.pop(99), 0, RuntimeError
)
dv = {i: i for i in range(10)}
moved(
    lambda: reversed(dv.items()),
    lambda: dv.update({99: 99}),
    lambda: dv.pop(99),
    0,
    RuntimeError,
)
