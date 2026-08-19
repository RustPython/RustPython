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
