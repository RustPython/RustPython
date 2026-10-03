"""Direct native-module probes; no dependency on the ported Python library."""

import _collections
import _elementtree
import sys

defaultdict = _collections.defaultdict
OrderedDict = _collections.OrderedDict
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


def default_union():
    left = defaultdict(int, [(1, 1), (2, 2)])
    right = frozendict([(0, "zero"), (1, "one")])
    expected = [(1, "one"), (2, 2), (0, "zero")]
    for rhs in (dict(right), right):
        check_result(left | rhs, defaultdict, expected, int)
    check_result(right | left, frozendict, [(0, "zero"), (1, 1), (2, 2)])
    check_result(left.__ror__(right), defaultdict, [(0, "zero"), (1, 1), (2, 2)], int)
    check_result(dict(right) | left, defaultdict, [(0, "zero"), (1, 1), (2, 2)], int)
    assert list(left.items()) == [(1, 1), (2, 2)]
    assert list(right.items()) == [(0, "zero"), (1, "one")]
    result = left | right
    result["fresh"] = 5
    assert "fresh" not in left and "fresh" not in right
    for factory in (None, list, str):
        empty = defaultdict(factory)
        check_result(empty | frozendict(), defaultdict, [], factory)
        check_result(empty.__ror__(frozendict()), defaultdict, [], factory)


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
    check_result(right | left, frozendict, [(0, 0), (1, 1), (2, 2)])


def ordered_union():
    left = OrderedDict([(0, 0), (1, 1), (2, 1)])
    right = frozendict([(1, 1), (2, 2), (3, 3)])
    check_result(left | right, OrderedDict, [(0, 0), (1, 1), (2, 2), (3, 3)])
    check_result(right | left, frozendict, [(1, 1), (2, 1), (3, 3), (0, 0)])
    check_result(left.__ror__(right), OrderedDict, [(1, 1), (2, 1), (3, 3), (0, 0)])
    assert list(left.items()) == [(0, 0), (1, 1), (2, 1)]
    check_result(OrderedDict() | frozendict(), OrderedDict, [])
    check_result(OrderedDict().__ror__(frozendict()), OrderedDict, [])


def ordered_subclass():
    calls = []

    class O(OrderedDict):
        def __init__(self, arg=()):
            calls.append(arg)
            super().__init__(arg)

    left = O([(1, 1), (2, 2)])
    right = frozendict([(0, 0), (1, 9)])
    calls.clear()
    check_result(left | right, O, [(1, 9), (2, 2), (0, 0)])
    assert len(calls) == 1 and calls[0] is left
    calls.clear()
    check_result(left.__ror__(right), O, [(0, 0), (1, 1), (2, 2)])
    assert len(calls) == 1 and calls[0] is right
    check_result(right | left, frozendict, [(0, 0), (1, 1), (2, 2)])


def union_operand_rejection_and_inplace():
    class Mapping:
        def keys(self):
            raise AssertionError("union must reject generic mappings before reading")

    for obj in (defaultdict(int, a=1), OrderedDict(a=1)):
        for invalid in (None, [], (), "", Mapping()):
            assert obj.__or__(invalid) is NotImplemented
            assert obj.__ror__(invalid) is NotImplemented
            raises(TypeError, lambda: obj | invalid)
            raises(TypeError, lambda: invalid | obj)
        identity = obj
        obj |= frozendict(b=2)
        obj |= [("a", 3), ("c", 4)]
        assert obj is identity and list(obj.items()) == [("a", 3), ("b", 2), ("c", 4)]
        if isinstance(obj, defaultdict):
            assert obj.default_factory is int


def frozen_mutation_rejection():
    frozen = frozendict(a=1)
    for action in (
        lambda: dict.__setitem__(frozen, "a", 2),
        lambda: dict.__delitem__(frozen, "a"),
        lambda: dict.update(frozen, {"a": 2}),
        lambda: dict.clear(frozen),
    ):
        raises(TypeError, action)
    assert frozen == {"a": 1}


def element_constructor():
    original = frozendict([("href", "#"), ("id", "old")])
    for make in (
        lambda: Element("a", original, id="new", extra="yes"),
        lambda: Element("a", attrib=original, id="new", extra="yes"),
        lambda: SubElement(
            Element("parent"), "a", attrib=original, id="new", extra="yes"
        ),
    ):
        element = make()
        assert element.tag == "a"
        assert type(element.attrib) is dict and element.attrib is not original
        assert list(element.attrib.items()) == [
            ("href", "#"),
            ("id", "new"),
            ("extra", "yes"),
        ]
        element.set("href", "changed")
        assert original["href"] == "#"
    both = Element("a", frozendict(a=1), attrib="extra attribute")
    assert both.attrib == {"a": 1, "attrib": "extra attribute"}


def element_empty_and_reinit():
    for make in (
        lambda: Element("a", frozendict()),
        lambda: Element("a", attrib=frozendict()),
        lambda: SubElement(Element("parent"), "a", attrib=frozendict()),
    ):
        one, two = make(), make()
        assert type(one.attrib) is dict and one.attrib == {}
        one.set("new", 1)
        assert two.attrib == {}
    obj = Element("old", {"keep": 1})
    original = obj.attrib
    obj.__init__("new", frozendict())
    assert obj.attrib is original and obj.attrib == {"keep": 1} and obj.tag == "new"
    obj.__init__("newer", attrib=frozendict(replace=2))
    assert obj.attrib is not original and obj.attrib == {"replace": 2}


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
    for obj in (Element("x", frozen), Element("x", attrib=frozen)):
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
    assert Element("x", attrib=nonempty).attrib == {"custom": value}
    assert Element("x", empty).attrib == {}
    assert Element("x", attrib=empty).attrib == {}


def element_argument_errors():
    raises(
        TypeError,
        lambda: Element("a", "bad"),
        "Element() argument 2 must be dict or frozendict, not str",
    )
    raises(
        TypeError,
        lambda: Element("a", attrib="bad"),
        "attrib must be dict or frozendict, not str",
    )
    raises(
        TypeError,
        lambda: SubElement(Element("p"), "a", "bad"),
        "SubElement() argument 3 must be dict, not str",
    )
    raises(
        TypeError,
        lambda: SubElement(Element("p"), "a", frozendict()),
        "SubElement() argument 3 must be dict, not frozendict",
    )
    raises(
        TypeError,
        lambda: SubElement(Element("p"), "a", attrib="bad"),
        "attrib must be dict or frozendict, not str",
    )
    raises(
        TypeError,
        lambda: Element("a").makeelement("b", frozendict()),
        "makeelement() argument 2 must be dict, not frozendict",
    )


def element_remove_error():
    parent, child = Element("parent"), Element("child")
    raises(
        ValueError,
        lambda: parent.remove(child),
        repr(child) + " not in " + repr(parent),
    )
    parent.append(child)
    assert parent.remove(child) is None and len(parent) == 0

    class Loud(Element):
        def __repr__(self):
            raise RuntimeError("repr failed")

    raises(RuntimeError, lambda: parent.remove(Loud("child")), "repr failed")


def main():
    cases = (
        default_union,
        default_subclass,
        ordered_union,
        ordered_subclass,
        union_operand_rejection_and_inplace,
        frozen_mutation_rejection,
        element_constructor,
        element_empty_and_reinit,
        element_shallow_copy,
        element_argument_errors,
        element_remove_error,
    )
    failures = 0
    for case in cases:
        try:
            case()
            print("PASS", case.__name__)
        except BaseException as exc:
            failures += 1
            print("FAIL", case.__name__, type(exc).__name__, str(exc))
    print("RESULT", len(cases) - failures, "passed", failures, "failed")
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
