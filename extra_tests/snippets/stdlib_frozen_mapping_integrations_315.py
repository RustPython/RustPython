"""Regressions for native frozen-map copying and subclass dispatch."""

import _collections
import _elementtree

defaultdict = _collections.defaultdict
Element = _elementtree.Element
SubElement = _elementtree.SubElement


def raises(exc_type, action, message=None):
    try:
        action()
    except exc_type as exc:
        if message is not None:
            assert str(exc) == message, (str(exc), message)
        return exc
    raise AssertionError("expected " + exc_type.__name__)


def check_result(result, cls, items, factory=None):
    assert type(result) is cls, (type(result), cls)
    assert list(result.items()) == items, list(result.items())
    if issubclass(cls, defaultdict):
        assert result.default_factory is factory


def default_subclass():
    calls = []

    class D(defaultdict):
        def __init__(self, factory=None, initial=()):
            calls.append((factory, initial))
            super().__init__(factory, initial)

        def update(self, *args, **kwargs):
            raise AssertionError("union must use native dict update")

        def __setitem__(self, key, value):
            raise AssertionError("union must not dispatch to overridden __setitem__")

    left = D(list, {1: 1, 2: 2})
    right = frozendict([(0, 0), (1, 9)])
    calls.clear()
    check_result(left | right, D, [(1, 9), (2, 2), (0, 0)], list)
    assert len(calls) == 1 and calls[0][0] is list and calls[0][1] is left
    calls.clear()
    check_result(left.__ror__(right), D, [(0, 0), (1, 1), (2, 2)], list)
    assert len(calls) == 1 and calls[0][0] is list and calls[0][1] is right


def element_shallow_copy():
    class Key:
        armed = False

        def __hash__(self):
            if self.armed:
                raise AssertionError("attribute copy must preserve cached key hashes")
            return 99

    class Frozen(frozendict):
        def keys(self):
            raise AssertionError("attribute copy must not call subclass keys")

        def __getitem__(self, key):
            raise AssertionError("attribute copy must not call subclass getitem")

    key, value = Key(), []
    frozen = Frozen({key: value})
    key.armed = True
    obj = Element("x", frozen)
    items = list(obj.attrib.items())
    assert type(obj.attrib) is dict and len(items) == 1
    assert items[0][0] is key and items[0][1] is value

    class CustomIter(frozendict):
        def __iter__(self):
            raise AssertionError("attribute copy dispatches through keys, not iter")

        def keys(self):
            return ["custom"]

        def __getitem__(self, key):
            assert key == "custom"
            return value

    nonempty, empty = CustomIter(original=1), CustomIter()
    assert Element("x", nonempty).attrib == {"custom": value}
    assert Element("x", empty).attrib == {}


def element_argument_errors():
    raises(
        TypeError,
        lambda: SubElement(Element("p"), "a", frozendict()),
        "SubElement() argument 3 must be dict, not frozendict",
    )


def element_remove_error():
    parent = Element("parent")

    class Loud(Element):
        def __repr__(self):
            raise RuntimeError("repr failed")

    raises(RuntimeError, lambda: parent.remove(Loud("child")), "repr failed")


default_subclass()
element_shallow_copy()
element_argument_errors()
element_remove_error()
