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


def check_length_hint_result_diagnostics():
    import sys

    class Example:
        class Result:
            pass

        class Source:
            def __init__(self, result):
                self.result = result

            def __iter__(self):
                return self

            def __next__(self):
                raise StopIteration

            def __len__(self):
                raise TypeError("try __length_hint__ instead")

            def __length_hint__(self):
                return self.result

    Example.Source.__module__ = "length_hint_example"
    Example.Result.__module__ = "length_hint_result"
    source_type = f"length_hint_example.{Example.Source.__qualname__}"
    result_type = f"length_hint_result.{Example.Result.__qualname__}"
    for consume in (_operator.length_hint, list, bytes):
        with assert_raises(TypeError) as caught:
            consume(Example.Source(Example.Result()))
        assert str(caught.exception) == (
            f"{source_type}.__length_hint__() must return an int, not {result_type}"
        )

        with assert_raises(ValueError) as caught:
            consume(Example.Source(-1))
        assert str(caught.exception) == (
            f"{source_type}.__length_hint__() must return a non-negative int"
        )

    for overrun in (sys.maxsize + 1, -sys.maxsize - 2):
        with assert_raises(OverflowError) as caught:
            _operator.length_hint(Example.Source(overrun))
        assert str(caught.exception) == "Python int too large to convert to C ssize_t"
        with assert_raises(OverflowError) as caught:
            _operator.length_hint(Example.Source(-1), overrun)
        assert str(caught.exception) == "Python int too large to convert to C ssize_t"


def check_length_hint_diagnostic_lifetimes():
    events = []

    class Result(int):
        def __del__(self):
            events.append("result")
            Source.__qualname__ = "SourceAfterResult"

    class Hint:
        def __call__(self):
            return Result(-1)

        def __del__(self):
            events.append("hint")
            Source.__qualname__ = "SourceAfterHint"

    class Descriptor:
        def __get__(self, instance, owner):
            return Hint()

    class Source:
        __module__ = "length_hint_example"
        __length_hint__ = Descriptor()

    with assert_raises(ValueError) as caught:
        _operator.length_hint(Source())
    assert events == ["hint", "result"]
    assert str(caught.exception) == (
        "length_hint_example.SourceAfterResult.__length_hint__() must return a non-negative int"
    )


check_length_hint_result_diagnostics()
check_length_hint_diagnostic_lifetimes()


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
