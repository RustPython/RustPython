import operator

from testutils import assert_raises


class Declines(str):
    def __add__(self, other):
        return NotImplemented


class Reflected:
    def __init__(self, result):
        self.calls = 0
        self.result = result

    def __radd__(self, other):
        self.calls += 1
        return self.result


class ReflectedStr(str):
    def __radd__(self, other):
        return "reflected"


def add(a, b):
    return a + b


def iadd(a, b):
    a += b
    return a


for operation in (add, iadd, operator.add, operator.iadd):
    assert operation("a", "b") == "ab"
    assert operation("a", Declines("b")) == "ab"
    assert_raises(TypeError, operation, Declines("a"), "b")
    assert operation("a", ReflectedStr("b")) == "reflected"
    for result in ("reflected", NotImplemented):
        other = Reflected(result)
        if result is NotImplemented:
            assert_raises(TypeError, operation, "a", other)
        else:
            assert operation("a", other) == result
        assert other.calls == 1

assert str.__add__("a", "b") == "ab"
assert str.__add__("a", ReflectedStr("b")) == "ab"
assert_raises(TypeError, str.__add__, "a", 1)
for result in ("reflected", NotImplemented):
    other = Reflected(result)
    assert_raises(TypeError, str.__add__, "a", other)
    assert other.calls == 0

assert not hasattr(str, "__radd__")

assert str.__add__(Declines("a"), "b") == "ab"


for base, left, right in (
    (str, "a", "b"),
    (bytes, b"a", b"b"),
    (tuple, (1,), (2,)),
    (list, [1], [2]),
):

    class Override(base):
        def __add__(self, other):
            return NotImplemented

    class Child(Override):
        pass

    for cls in (Override, Child):
        assert_raises(TypeError, operator.add, cls(left), right)

    Override.__add__ = base.__add__
    for cls in (Override, Child):
        assert operator.add(cls(left), right) == left + right

    Override.__add__ = lambda self, other: "override"
    for cls in (Override, Child):
        assert operator.add(cls(left), right) == "override"

    del Override.__add__
    for cls in (Override, Child):
        assert operator.add(cls(left), right) == left + right


# Inheriting native concatenation must leave reflected numeric addition first.
for base, left in (
    (str, "a"),
    (bytes, b"a"),
    (tuple, (1,)),
    (list, [1]),
    (bytearray, b"a"),
):

    class Native(base):
        pass

    class Child(Native):
        pass

    class Explicit(base):
        __add__ = base.__add__

    for cls in (Native, Child, Explicit):
        for result in ("reflected", NotImplemented):
            other = Reflected(result)
            if result is NotImplemented:
                assert_raises(TypeError, operator.add, cls(left), other)
            else:
                assert operator.add(cls(left), other) == result
            assert other.calls == 1

    class ReflectedSequence(base):
        calls = 0

        def __radd__(self, other):
            self.calls += 1
            return NotImplemented

    other = ReflectedSequence(left)
    assert operator.add(Native(left), other) == base(left) + base(left)
    assert other.calls == 1

    class AddOverride(ReflectedSequence):
        def __add__(self, other):
            return "overridden"

    other = AddOverride(left)
    assert operator.add(ReflectedSequence(left), other) == base(left) + base(left)
    assert other.calls == 0
    del AddOverride.__add__
    other = AddOverride(left)
    assert operator.add(ReflectedSequence(left), other) == base(left) + base(left)
    assert other.calls == 0

    # CPython keeps numeric dispatch after either addition method is overridden,
    # including when deletion or replacement exposes the native __add__ again.
    Native.__radd__ = lambda self, other: NotImplemented
    for change in (
        lambda: None,
        lambda: delattr(Native, "__radd__"),
        lambda: setattr(Native, "__add__", base.__add__),
        lambda: delattr(Native, "__add__"),
    ):
        change()
        for cls in (Native, Child):
            other = Reflected("reflected")
            assert_raises(TypeError, operator.add, cls(left), other)
            assert other.calls == 0


# A differently named descriptor assigned to __add__ needs numeric dispatch.
class InplaceAlias(list):
    __add__ = list.__iadd__


other = Reflected("reflected")
assert_raises(TypeError, operator.add, InplaceAlias([1]), other)
assert other.calls == 0


class NativeIntAdd(int):
    __add__ = int.__add__

    def __radd__(self, other):
        raise AssertionError("inherited reflected addition took priority")


class IntAddOverride(NativeIntAdd):
    def __add__(self, other):
        return "overridden"


assert operator.add(NativeIntAdd(1), IntAddOverride(2)) == 3
