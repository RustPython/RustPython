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

# Repeating a bounded deque keeps only the last maxlen items without walking the rest
assert deque([1], maxlen=3) * 2**62 == deque([1, 1, 1])
assert deque([1, 2, 3], maxlen=4) * 3 == deque([3, 1, 2, 3])


def assert_deque_error(error_type, message, function, *args):
    with assert_raises(error_type) as caught:
        function(*args)
    assert type(caught.exception) is error_type, type(caught.exception)
    assert str(caught.exception) == message, str(caught.exception)


def test_deque_remove_detects_deletion_during_comparison():
    for match in (True, False):
        d = deque()

        class Item:
            def __eq__(self, other):
                del d[0]
                return match

        d.append(Item())
        assert_deque_error(IndexError, "deque mutated during iteration", d.remove, None)
        assert not d


def test_deque_remove_stops_after_comparison_appends():
    d = deque()

    class Item:
        calls = 0

        def __eq__(self, other):
            self.calls += 1
            assert self.calls == 1, "remove continued after mutation"
            d.append(self)
            return False

    item = Item()
    d.append(item)
    assert_deque_error(IndexError, "deque mutated during iteration", d.remove, None)
    assert item.calls == 1 and len(d) == 2


def test_deque_remove_preserves_identity_first_match_and_return_value():
    class Identity:
        def __eq__(self, other):
            raise AssertionError("identity should not compare")

    class Unequal:
        calls = 0

        def __eq__(self, other):
            self.calls += 1
            return False

    identity, unequal = Identity(), Unequal()
    d = deque([identity, unequal, 1, 2, 1])
    assert d.remove(identity) is None
    assert d.remove(1) is None
    assert unequal.calls == 1
    assert list(d) == [unequal, 2, 1]
    assert_deque_error(ValueError, "deque.remove(x): x not in deque", d.remove, 3)
    assert list(d) == [unequal, 2, 1]


def test_deque_deletion_invalidates_iterators_only_after_success():
    for factory in (iter, reversed):
        for index in (0, 1, -1):
            d = deque([1, 2, 3])
            iterator = factory(d)
            del d[index]
            assert_deque_error(
                RuntimeError, "deque mutated during iteration", next, iterator
            )
        d = deque([1, 2, 3])
        iterator = factory(d)

        def delete(index):
            del d[index]

        assert_deque_error(IndexError, "deque index out of range", delete, 10)
        assert list(iterator) == list(factory(d))
        iterator = factory(d)
        assert d.remove(2) is None
        assert_deque_error(
            RuntimeError, "deque mutated during iteration", next, iterator
        )


def test_deque_remove_preserves_comparison_error_after_mutation():
    d = deque()

    class Item:
        def __eq__(self, other):
            del d[0]
            raise LookupError("comparison failed")

    d.append(Item())
    assert_deque_error(LookupError, "comparison failed", d.remove, None)
    assert not d


def test_deque_deletion_releases_lock_before_finalizing_item():
    d = deque()

    class Item:
        def __del__(self):
            d.append("finalized")

    d.append(Item())
    del d[0]
    assert list(d) == ["finalized"]


def test_deque_remove_detects_mutation_from_compared_item_finalizer():
    for match in (True, False):
        d = deque()

        class Item:
            def __eq__(self, other):
                d[0] = "replacement"
                return match

            def __del__(self):
                d.append("finalized")

        d.append(Item())
        assert_deque_error(IndexError, "deque mutated during iteration", d.remove, None)
        assert list(d) == ["replacement", "finalized"]


test_deque_remove_detects_deletion_during_comparison()
test_deque_remove_stops_after_comparison_appends()
test_deque_remove_preserves_identity_first_match_and_return_value()
test_deque_deletion_invalidates_iterators_only_after_success()
test_deque_remove_preserves_comparison_error_after_mutation()
test_deque_deletion_releases_lock_before_finalizing_item()
test_deque_remove_detects_mutation_from_compared_item_finalizer()


def test_deque_remove_detects_bulk_mutation():
    operations = (
        lambda d: d.extendleft([1]),
        lambda d: d.__imul__(0),
        lambda d: d.__imul__(2),
        lambda d: d.__init__([1]),
    )
    for operation in operations:
        for match in (True, False):
            d = deque()

            class Item:
                calls = 0

                def __eq__(self, other):
                    self.calls += 1
                    assert self.calls == 1, "remove continued after mutation"
                    operation(d)
                    return match

            d.append(Item())
            assert_deque_error(
                IndexError, "deque mutated during iteration", d.remove, None
            )


def test_deque_bulk_mutation_invalidates_iterators():
    operations = (
        lambda d: d.extendleft([9]),
        lambda d: d.__imul__(0),
        lambda d: d.__imul__(2),
        lambda d: d.__init__([9]),
    )
    for factory in (iter, reversed):
        for maxlen in (None, 1, 3):
            for operation in operations:
                d = deque([1, 2], maxlen=maxlen)
                iterator = factory(d)
                operation(d)
                assert_deque_error(
                    RuntimeError, "deque mutated during iteration", next, iterator
                )


def test_deque_bulk_noops_preserve_iterators():
    for factory in (iter, reversed):
        for items in ([], [1, 2]):
            for operation in (lambda d: d.extendleft([]), lambda d: d.__imul__(1)):
                d = deque(items)
                iterator = factory(d)
                operation(d)
                assert list(iterator) == list(factory(d))
        for count in (-1, 0, 2):
            d = deque()
            iterator = factory(d)
            d *= count
            assert list(iterator) == []
        d = deque(maxlen=0)
        iterator = factory(d)
        d.extendleft([1, 2])
        assert list(iterator) == []
        d = deque()
        iterator = factory(d)
        d.__init__([])
        assert list(iterator) == []


def test_deque_bulk_mutation_releases_lock_before_finalizing():
    operations = (
        (lambda d: d.extendleft([9]), [9], ["finalized"]),
        (lambda d: d.__imul__(0), [], ["finalized"]),
        (lambda d: d.__init__([9]), [], ["finalized", 9]),
    )
    for operation, observed, expected in operations:
        d = deque(maxlen=1)
        snapshots = []

        class Item:
            def __del__(self):
                snapshots.append(list(d))
                d.append("finalized")

        d.append(Item())
        operation(d)
        assert snapshots == [observed], snapshots
        assert list(d) == expected, d


def test_deque_reinitialization_order_and_limit():
    d = deque([1, 2, 3])
    d.__init__(d)
    assert not d
    d = deque([1, 2, 3])
    iterator = iter(d)
    assert_raises(ValueError, d.__init__, [], -1)
    assert list(iterator) == [1, 2, 3]
    iterator = iter(d)
    assert_raises(TypeError, d.__init__, 1)
    assert not d
    assert_deque_error(RuntimeError, "deque mutated during iteration", next, iterator)

    for maxlen, expected in ((1, [9]), (2, ["finalized", 9])):
        d = deque()
        snapshots = []

        class Item:
            def __del__(self):
                snapshots.append((list(d), d.maxlen))
                d.append("finalized")

        d.append(Item())
        d.__init__([9], maxlen)
        assert snapshots == [([], maxlen)], snapshots
        assert list(d) == expected, d


test_deque_remove_detects_bulk_mutation()
test_deque_bulk_mutation_invalidates_iterators()
test_deque_bulk_noops_preserve_iterators()
test_deque_bulk_mutation_releases_lock_before_finalizing()
test_deque_reinitialization_order_and_limit()


def test_deque_streaming_preserves_prefix_on_error():
    for method in ("extend", "extendleft", "__iadd__", "__init__"):
        d = deque([10], maxlen=2)
        seen = []

        def source():
            yield 1
            seen.append(list(d))
            yield 2
            raise ValueError("source failed")

        if method == "__init__":
            assert_raises(ValueError, d.__init__, source(), 2)
            expected_seen = [1]
        else:
            assert_raises(ValueError, getattr(d, method), source())
            expected_seen = [1, 10] if method == "extendleft" else [10, 1]
        assert seen == [expected_seen], (method, seen)
        assert list(d) == ([2, 1] if method == "extendleft" else [1, 2])


def test_deque_streaming_keeps_only_bounded_items():
    for operation in ("constructor", "extend", "extendleft"):
        for maxlen in (0, 3):

            class Item:
                live = 0
                peak = 0

                def __init__(self):
                    Item.live += 1
                    Item.peak = max(Item.peak, Item.live)

                def __del__(self):
                    Item.live -= 1

            source = (Item() for _ in range(20))
            if operation == "constructor":
                d = deque(source, maxlen=maxlen)
            else:
                d = deque(maxlen=maxlen)
                getattr(d, operation)(source)
            assert Item.peak <= maxlen + 1, (operation, maxlen, Item.peak)
            assert Item.live == maxlen, (operation, maxlen, Item.live)
            del d
            assert Item.live == 0


def test_deque_self_extension_uses_subclass_iterator():
    class CustomDeque(deque):
        def __iter__(self):
            return iter((7, 8))

    for method in ("extend", "extendleft", "__iadd__"):
        d = CustomDeque([1, 2])
        getattr(d, method)(d)
        expected = [8, 7, 1, 2] if method == "extendleft" else [1, 2, 7, 8]
        assert list(deque.__iter__(d)) == expected, method

    d = deque([1, 2])
    assert_raises(RuntimeError, d.extend, iter(d))
    assert list(d) == [1, 2, 1]


def test_deque_extend_noops_preserve_iterators():
    d = deque([1, 2])
    iterator = iter(d)
    d.extend(iter(()))
    assert_raises(TypeError, d.extend, None)
    assert list(iterator) == [1, 2]

    d = deque(maxlen=0)
    iterator = iter(d)
    d.extend(iter((1, 2)))
    assert list(iterator) == []


def test_deque_self_extension_retains_snapshot_until_complete():
    for method in ("extend", "extendleft"):
        seen = []

        class Item:
            def __init__(self, value):
                self.value = value

            def __del__(self):
                seen.append([item.value for item in deque.__iter__(d)])

        class CustomDeque(deque):
            def __iter__(self):
                return (Item(value) for value in range(3))

        d = CustomDeque(maxlen=1)
        getattr(d, method)(d)
        assert seen == [[2], [2]], (method, seen)
        last = d[0]
        d.clear()
        del last


def test_deque_eviction_can_mutate_source_list():
    for method in ("extend", "extendleft"):
        d = deque(maxlen=1)
        source = [1, 2]
        seen = []

        class Item:
            def __del__(self):
                seen.append(list(d))
                source[1:] = [3, 4]

        d.append(Item())
        getattr(d, method)(source)
        assert seen == [[1]], (method, seen)
        assert list(d) == [4], (method, d)


test_deque_streaming_preserves_prefix_on_error()
test_deque_streaming_keeps_only_bounded_items()
test_deque_self_extension_uses_subclass_iterator()
test_deque_extend_noops_preserve_iterators()
test_deque_self_extension_retains_snapshot_until_complete()
test_deque_eviction_can_mutate_source_list()
