import _operator

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
