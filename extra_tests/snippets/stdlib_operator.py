import _operator
from collections import Counter

from testutils import assert_raises

assert _operator._compare_digest("abcdef", "abcdef")
assert not _operator._compare_digest("abcdef", "abc")
assert not _operator._compare_digest("abc", "abcdef")


def check_operator_reference_cycles():
    import gc
    import weakref

    class Holder:
        pass

    class Name(str):
        pass

    for make, make_payload in (
        (_operator.itemgetter, Holder),
        (lambda value: _operator.methodcaller("method", value), Holder),
        (lambda value: _operator.methodcaller("method", value=value), Holder),
        (_operator.attrgetter, lambda: Name("attribute")),
    ):
        payload = make_payload()
        wrapper = make(payload)
        payload.wrapper = wrapper
        reference = weakref.ref(payload)
        del wrapper, payload
        gc.collect()
        assert reference() is None


check_operator_reference_cycles()


class Index:
    def __index__(self):
        return 5


class RaisingIndex:
    def __index__(self):
        raise ZeroDivisionError


class TupleHint:
    def __length_hint__(self):
        return ()


assert _operator.length_hint(object(), Index()) == 5
assert_raises(ZeroDivisionError, _operator.length_hint, [], RaisingIndex())
assert _operator.length_hint(object(), -1) == -1
assert _operator.length_hint([], -1) == 0
assert_raises(TypeError, _operator.length_hint, TupleHint(), -1)
assert_raises(OverflowError, _operator.length_hint, object(), -(2**70))


class AddsAnything:
    def __getitem__(self, index):
        return 0

    def __add__(self, other):
        return "added"

    def __iadd__(self, other):
        return "iadded"


class ListWithIadd(list):
    def __iadd__(self, other):
        return "iadded"


class ListWithBoth(list):
    def __add__(self, other):
        return "added"

    def __iadd__(self, other):
        return "iadded"


assert _operator.concat(AddsAnything(), [1]) == "added"
assert_raises(TypeError, _operator.concat, AddsAnything(), 1)
assert _operator.iconcat(AddsAnything(), [1]) == "iadded"
assert_raises(TypeError, _operator.iconcat, AddsAnything(), 1)
receiver = ListWithIadd([1])
result = _operator.iconcat(receiver, [2])
assert type(result) is list
assert result == [1, 2]
assert receiver == [1]
assert _operator.iconcat(ListWithBoth([1]), [2]) == "iadded"
assert_raises(TypeError, _operator.concat, Counter(a=1), Counter(b=1))
assert_raises(TypeError, _operator.iconcat, Counter(a=1), Counter(b=1))
