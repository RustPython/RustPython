from array import array
from collections import deque
from datetime import date, timedelta
from types import MethodWrapperType, WrapperDescriptorType

from testutils import assert_raises

# Native #[pymethod] __repr__ implementations must expose the descriptor for
# tp_repr, not an ordinary method that happens to produce the same text.
cases = [
    (int, 42, "42"),
    (bool, True, "True"),
    (float, 1.5, "1.5"),
    (complex, 1 + 2j, "(1+2j)"),
    (str, "\ud800", "'\\ud800'"),
    (bytes, b"abc", "b'abc'"),
    (bytearray, bytearray(b"abc"), "bytearray(b'abc')"),
    (list, [1], "[1]"),
    (tuple, (1,), "(1,)"),
    (dict, {"a": 1}, "{'a': 1}"),
    (set, {1}, "{1}"),
    (frozenset, frozenset({1}), "frozenset({1})"),
    (range, range(3), "range(0, 3)"),
    (BaseException, BaseException("error"), "BaseException('error')"),
    (array, array("i", [1, 2]), "array('i', [1, 2])"),
    (deque, deque([1, 2]), "deque([1, 2])"),
    (date, date(2026, 1, 2), "datetime.date(2026, 1, 2)"),
    (timedelta, timedelta(days=2), "datetime.timedelta(days=2)"),
]
for cls, value, expected in cases:
    descriptor = cls.__dict__["__repr__"]
    assert type(descriptor) is WrapperDescriptorType
    assert descriptor.__objclass__ is cls
    assert descriptor.__name__ == "__repr__"
    assert type(value.__repr__) is MethodWrapperType
    assert repr(value) == descriptor(value) == value.__repr__() == expected
    with assert_raises(TypeError):
        descriptor(object())


class Subclass(list):
    pass


value = Subclass([1])
value.__repr__ = lambda: "instance shadow"
assert value.__repr__() == "instance shadow"
assert repr(value) == "[1]"
Subclass.__repr__ = lambda self: "class override"
assert repr(value) == "class override"
assert list.__repr__(value) == "[1]"
del Subclass.__repr__
assert repr(value) == "[1]"

# Keep recursion guards and representation errors on the native slot path.
recursive_list = []
recursive_list.append(recursive_list)
assert repr(recursive_list) == "[[...]]"
recursive_dict = {}
recursive_dict["self"] = recursive_dict
assert repr(recursive_dict) == "{'self': {...}}"


class BadRepr:
    def __repr__(self):
        raise ValueError("repr failed")


with assert_raises(ValueError):
    repr([BadRepr()])

# Dict-view repr was supplied by a shared trait implementation.
for value in [{"a": 1}.keys(), {"a": 1}.values(), {"a": 1}.items()]:
    descriptor = type(value).__dict__["__repr__"]
    assert type(descriptor) is WrapperDescriptorType
    assert repr(value) == descriptor(value)
