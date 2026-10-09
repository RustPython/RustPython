from collections import deque

d = deque([0, 1, 2])

d.append(1)
d.appendleft(3)

assert d == deque([3, 0, 1, 2, 1])

assert d <= deque([4])

assert d.copy() is not d

d = deque([1, 2, 3], 5)

d.extend([4, 5, 6])

assert d == deque([2, 3, 4, 5, 6]), d

d.remove(4)

assert d == deque([2, 3, 5, 6])

d.clear()

assert d == deque()

assert d == deque([], 4)

assert deque([1, 2, 3]) * 2 == deque([1, 2, 3, 1, 2, 3])

assert deque([1, 2, 3], 4) * 2 == deque([3, 1, 2, 3])

assert deque(maxlen=3) == deque()

assert deque([1, 2, 3, 4], maxlen=2) == deque([3, 4])

assert len(deque([1, 2, 3, 4])) == 4

assert d >= d
assert not (d > d)
assert d <= d
assert not (d < d)
assert d == d
assert not (d != d)


# Test that calling an evil __repr__ can't hang deque
class BadRepr:
    def __repr__(self):
        self.d.pop()
        return ""


b = BadRepr()
d = deque([1, b, 2])
b.d = d
repr(d)

# namedtuple fields are `_tuplegetter` descriptors
import pickle
from _collections import _count_elements, _tuplegetter
from collections import Counter, OrderedDict, defaultdict, namedtuple

from testutils import assert_raises

Point = namedtuple("Point", "x y")
p = Point(1, 2)
assert type(Point.x) is _tuplegetter
assert (p.x, p.y) == (1, 2)
assert repr(Point.x) == "_tuplegetter(0, 'Alias for field number 0')"
assert Point.x.__get__(None, Point) is Point.x
assert Point.y.__get__((5, 6)) == 6
assert pickle.loads(pickle.dumps(Point.y)).__get__((5, 6)) == 6
with assert_raises(TypeError):
    Point.x.__get__([1, 2])
with assert_raises(IndexError):
    _tuplegetter(3, None).__get__((1, 2))
with assert_raises(AttributeError):
    p.x = 3
with assert_raises(AttributeError):
    del p.x
Point.x.__doc__ = "The x-coordinate"
assert repr(Point.x) == "_tuplegetter(0, 'The x-coordinate')"

# `_count_elements` behind Counter
counts = {}
_count_elements(counts, "abca")
assert counts == {"a": 2, "b": 1, "c": 1}
assert Counter("abracadabra").most_common(1) == [("a", 5)]
ordered = OrderedDict()
_count_elements(ordered, "bab")
assert list(ordered.items()) == [("b", 2), ("a", 1)]
factory = defaultdict(lambda: 100)
_count_elements(factory, "a")
assert factory == {"a": 1}


class Scaled(dict):
    def __setitem__(self, key, value):
        super().__setitem__(key, value * 10)


scaled = Scaled()
_count_elements(scaled, "aab")
assert scaled == {"a": 110, "b": 10}


class CustomGet(dict):
    def get(self, key, default=None):
        return 100


custom_get = CustomGet()
_count_elements(custom_get, "aab")
assert custom_get == {"a": 101, "b": 101}
with assert_raises(AttributeError):
    _count_elements([], "a")


# defaultdict union constructs the subclass with its factory and left operand.
def check_defaultdict_subclass_union():
    calls = []

    class DefaultSubclass(defaultdict):
        def __init__(self, factory=None, initial=()):
            calls.append((factory, initial))
            super().__init__(factory, initial)

        def update(self, *args, **kwargs):
            raise AssertionError("union must use native dict update")

        def __setitem__(self, key, value):
            raise AssertionError("union must not call overridden __setitem__")

    for factory in (None, list):
        left = DefaultSubclass(factory, {"shared": 1, "left": 2})
        right = {"right": 3, "shared": 4}
        calls.clear()
        result = left | right
        assert type(result) is DefaultSubclass
        assert result.default_factory is factory
        assert list(result.items()) == [("shared", 4), ("left", 2), ("right", 3)]
        assert len(calls) == 1 and calls[0][0] is factory and calls[0][1] is left

        calls.clear()
        result = right | left
        assert type(result) is DefaultSubclass
        assert result.default_factory is factory
        assert list(result.items()) == [("right", 3), ("shared", 1), ("left", 2)]
        assert len(calls) == 1 and calls[0][0] is factory and calls[0][1] is right
        assert left == {"shared": 1, "left": 2}
        assert right == {"right": 3, "shared": 4}

        calls.clear()
        assert left.__or__([("x", 1)]) is NotImplemented
        assert left.__ror__([("x", 1)]) is NotImplemented
        assert calls == []

    class FailingSubclass(defaultdict):
        fail = False

        def __init__(self, *args):
            if self.fail:
                raise ValueError("union constructor")
            super().__init__(*args)

    left = FailingSubclass(int, {"a": 1})
    FailingSubclass.fail = True
    with assert_raises(ValueError) as caught:
        left | {}
    assert str(caught.exception) == "union constructor"
    with assert_raises(ValueError) as caught:
        {} | left
    assert str(caught.exception) == "union constructor"

    class ReplacingSubclass(defaultdict):
        replacement = None

        def __new__(cls, *args):
            if cls.replacement is not None:
                return cls.replacement
            return super().__new__(cls)

    left = ReplacingSubclass(int, {"left": 1})
    replacement = {"replacement": 2}
    ReplacingSubclass.replacement = replacement
    assert left | {"right": 3} is replacement
    assert replacement == {"replacement": 2, "right": 3}
    replacement.clear()
    assert {"right": 3} | left is replacement
    assert replacement == {"left": 1}
    ReplacingSubclass.replacement = object()
    with assert_raises(SystemError):
        left | {}
    with assert_raises(SystemError):
        {} | left


check_defaultdict_subclass_union()
