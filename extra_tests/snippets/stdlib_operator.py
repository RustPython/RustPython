import _operator

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
