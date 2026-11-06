import _bisect
import functools
import gc
import sys
import weakref
from bisect import bisect_left, bisect_right, insort, insort_left, insort_right

# A key decides where the item goes, but the item is what gets stored.
for insort_fn in (insort, insort_left, insort_right):
    words = ["a", "ccc"]
    insort_fn(words, "bb", key=len)
    assert words == ["a", "bb", "ccc"], (insort_fn.__name__, words)

    numbers = [1, 3]
    insort_fn(numbers, -2, key=abs)
    assert numbers == [1, -2, 3], (insort_fn.__name__, numbers)

    pairs = [(1, "a"), (3, "b")]
    insort_fn(pairs, (2, "x"), key=lambda pair: pair[0])
    assert pairs == [(1, "a"), (2, "x"), (3, "b")], (insort_fn.__name__, pairs)


descending = [3, 1]
insort(descending, 2, key=lambda value: -value)
assert descending == [3, 2, 1], descending


class Tagged:
    def __init__(self, size, tag):
        self.size = size
        self.tag = tag


def sizes_and_tags(items):
    return [(item.size, item.tag) for item in items]


by_size = [Tagged(1, "old"), Tagged(2, "old")]

# On a tie, left goes before the equal element and right goes after it.
left = list(by_size)
insort_left(left, Tagged(2, "new"), key=lambda item: item.size)
assert sizes_and_tags(left) == [(1, "old"), (2, "new"), (2, "old")]

right = list(by_size)
insort_right(right, Tagged(2, "new"), key=lambda item: item.size)
assert sizes_and_tags(right) == [(1, "old"), (2, "old"), (2, "new")]


# The key runs once on the new item, whatever the search does afterwards.
seen = []


def counting_key(value):
    seen.append(value)
    return value


data = [1, 3, 5, 7]
insort(data, 4, key=counting_key)
assert data == [1, 3, 4, 5, 7], data
assert seen.count(4) == 1, seen


# lo and hi bound the search and leave the inserted item alone.
bounded = [3, 1]
insort(bounded, 2, 0, 1, key=lambda value: -value)
assert bounded == [3, 2, 1], bounded


plain = [1, 3]
insort(plain, 2)
assert plain == [1, 2, 3], plain

# The search takes the key value itself, so these two stay where they were.
sizes = ["a", "bb", "ccc"]
assert bisect_left(sizes, 2, key=len) == 1
assert bisect_right(sizes, 2, key=len) == 2


# Clinic rejects malformed calls before converting bounds, and finishes lo's
# validation before starting hi's conversion.
class Index:
    def __init__(self, name, value):
        self.name = name
        self.value = value

    def __index__(self):
        events.append(self.name)
        return self.value


for fn in (
    _bisect.bisect_left,
    _bisect.bisect_right,
    _bisect.insort_left,
    _bisect.insort_right,
):
    events = []
    try:
        fn([], 0, Index("lo", 0), unknown=True)
    except TypeError:
        pass
    else:
        raise AssertionError("unknown keyword accepted")
    assert events == [], events

    for value, error, message in (
        (-1, ValueError, "lo cannot be negative"),
        (
            sys.maxsize + 1,
            OverflowError,
            "Python int too large to convert to C ssize_t",
        ),
    ):
        events = []
        try:
            fn([], 0, Index("lo", value), Index("hi", 0))
        except error as exc:
            assert str(exc) == message, str(exc)
        else:
            raise AssertionError("invalid lo accepted")
        assert events == ["lo"], events

    events = []
    try:
        fn([], 0, hi=Index("hi", sys.maxsize + 1))
    except OverflowError as exc:
        assert str(exc) == "cannot fit 'Index' into an index-sized integer", str(exc)
    else:
        raise AssertionError("overflowing hi accepted")
    assert events == ["hi"], events

    try:
        fn([], 0, hi=object())
    except TypeError as exc:
        assert str(exc) == "argument should be integer or None, not 'object'", str(exc)
    else:
        raise AssertionError("non-index hi accepted")

    for methods, error, template in (
        ({}, TypeError, "argument should be integer or None, not '{}'"),
        (
            {"__index__": lambda self: sys.maxsize + 1},
            OverflowError,
            "cannot fit '{}' into an index-sized integer",
        ),
    ):
        LongName = type("A" * 201, (), methods)
        try:
            fn([], 0, hi=LongName())
        except error as exc:
            assert str(exc) == template.format("A" * 200), str(exc)
        else:
            raise AssertionError("invalid hi accepted")

    # No Python frame is entered by the recursive comparison. The search must
    # guard its native same-type shortcut rather than relying on generic richcmp.
    class Recursive:
        pass

    recursive_item = Recursive()
    recursive_items = [recursive_item]
    Recursive.__lt__ = staticmethod(functools.partial(fn, recursive_items))
    try:
        fn(recursive_items, recursive_item)
    except RecursionError:
        pass
    else:
        raise AssertionError("native bisect recursion was not stopped")


# Keeping a cached comparison slot must not retain the old needle class after
# a callback changes __class__ and collects it.
for fn in (_bisect.bisect_left, _bisect.bisect_right):
    events = []

    class Replacement:
        def __lt__(self, other):
            events.append("compare")
            return False

    Original = type("Original", (), {"__lt__": lambda self, other: False})
    needle = Original()
    old_class = weakref.ref(Original, lambda ref: events.append("collected"))
    del Original

    def replace_class():
        # RustPython retains a reassigned class until the assigning frame exits.
        # Isolate that lifetime from the search's cached type ownership.
        needle.__class__ = Replacement

    class Sequence:
        def __len__(self):
            return 1

        def __getitem__(self, index):
            replace_class()
            gc.collect()
            events.append("getitem")
            return Replacement()

    fn(Sequence(), needle)
    assert events == ["collected", "getitem", "compare"], events


# insort discards a custom insert result while all original arguments are alive.
for fn in (_bisect.insort_left, _bisect.insort_right):
    events = []

    class Item:
        def __del__(self):
            events.append("item-del")

    class InsertResult:
        def __del__(self):
            events.append("result-del")

    class Sequence:
        def __getitem__(self, index):
            raise AssertionError("an empty interval must not fetch an item")

        def insert(self, index, item):
            events.append("insert")
            return InsertResult()

    assert fn(Sequence(), Item(), hi=0) is None
    assert events == ["insert", "result-del", "item-del"], events
