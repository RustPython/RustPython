import operator

from testutils import assert_raises


def check_comparison():
    for compare, expected in (
        (operator.eq, False),
        (operator.ne, True),
        (operator.lt, True),
        (operator.le, True),
        (operator.gt, False),
        (operator.ge, False),
    ):
        target = bytearray(b"a")
        events = []

        class Other:
            def __buffer__(self, flags):
                events.append("acquire")
                with assert_raises(BufferError):
                    target.__init__(object())
                target[0] = ord("A")
                return memoryview(b"B")

            def __release_buffer__(self, view):
                events.append("release")
                target[0] = ord("Z")
                target.append(99)

        assert compare(target, Other()) is expected
        assert target == b"Zc"
        assert events == ["acquire", "release"]

    events = []

    class Left(bytearray):
        def __buffer__(self, flags):
            events.append("acquire")
            return memoryview(b"L")

        def __release_buffer__(self, view):
            events.append("release")

    left = Left(b"ignored")
    assert left.__eq__(left) is True
    assert events == ["acquire", "acquire", "release", "release"]
    events.clear()
    assert left.__eq__(object()) is NotImplemented
    assert events == []

    class Failing:
        def __buffer__(self, flags):
            raise ValueError("no buffer")

    assert left.__eq__(Failing()) is NotImplemented
    assert events == ["acquire", "release"]
    left.append(0)


def check_search():
    for method, expected in (
        ("find", 0),
        ("index", 0),
        ("rfind", 2),
        ("rindex", 2),
        ("count", 2),
    ):
        target = bytearray(b"xxx")
        events = []

        class Needle:
            def __buffer__(self, flags):
                events.append("acquire")
                with assert_raises(BufferError):
                    target.__init__(object())
                target[:] = b"aba"
                return memoryview(b"a")

            def __release_buffer__(self, view):
                events.append("release")
                with assert_raises(BufferError):
                    target.append(0)
                target[:] = b"zzz"

        assert getattr(target, method)(Needle()) == expected
        assert target == b"zzz"
        assert events == ["acquire", "release"]
        target.append(0)

        target = bytearray(b"xxx")

        class IndexNeedle:
            def __index__(self):
                with assert_raises(BufferError):
                    target.__init__(object())
                target[:] = b"aba"
                return ord("a")

        assert getattr(target, method)(IndexNeedle()) == expected

    target = bytearray(b"ab")

    class Start:
        def __index__(self):
            target.extend(b"ab")
            return 1

    assert target.count(ord("a"), Start()) == 1
    assert target == b"abab"


def check_affixes():
    for method in ("startswith", "endswith"):
        target = bytearray(b"aaa")
        events = []

        class Affix:
            def __init__(self, value, after):
                self.value = value
                self.after = after

            def __buffer__(self, flags):
                events.append(self.value)
                with assert_raises(BufferError):
                    target.__init__(object())
                target[0] = target[0]
                return memoryview(self.value)

            def __release_buffer__(self, view):
                events.append("release")
                with assert_raises(BufferError):
                    target.append(0)
                target[:] = self.after

        class Unvisited:
            def __buffer__(self, flags):
                raise AssertionError("tuple must short-circuit")

        assert getattr(target, method)(
            (Affix(b"x", b"yyy"), Affix(b"y", b"zzz"), Unvisited())
        )
        assert target == b"zzz"
        assert events == [b"x", "release", b"y", "release"]
        target.append(0)

        target = bytearray(b"aaa")
        events.clear()
        assert getattr(target, method)(Affix(b"a", b"zzz"))
        assert target == b"zzz"
        assert events == [b"a", "release"]

        events.clear()
        assert not getattr(target, method)(Affix(b"z", b"aaa"), 100)
        assert events == [b"z", "release"]
        with assert_raises(TypeError):
            getattr(target, method)((Affix(b"x", b"aaa"), object()))
        target.append(0)

        class BytesAffix(bytes):
            def __buffer__(self, flags):
                raise AssertionError("bytes subclasses use their stored bytes")

        assert getattr(bytearray(b"a"), method)(BytesAffix(b"a"))


def check_translate():
    target = bytearray(b"x")
    table = bytearray(range(256))
    events = []

    class Table:
        def __buffer__(self, flags):
            events.append("table acquire")
            target.__init__(b"a")
            return memoryview(table)

        def __release_buffer__(self, view):
            events.append("table release")
            target.__init__(b"t")

    class Delete:
        def __buffer__(self, flags):
            events.append("delete acquire")
            with assert_raises(BufferError):
                table.append(0)
            table[ord("a")] = ord("b")
            target.__init__(b"aa")
            return memoryview(b"")

        def __release_buffer__(self, view):
            events.append("delete release")
            target.__init__(b"d")

    assert target.translate(Table(), delete=Delete()) == b"bb"
    assert target == b"d"
    assert events == [
        "table acquire",
        "delete acquire",
        "table release",
        "delete release",
    ]
    table.append(0)
    table.pop()
    events.clear()

    class BadDelete:
        def __buffer__(self, flags):
            events.append("delete failure")
            with assert_raises(BufferError):
                table.append(0)
            raise ValueError("bad delete")

    with assert_raises(ValueError):
        target.translate(Table(), delete=BadDelete())
    assert events == ["table acquire", "delete failure", "table release"]
    table.append(0)


def check_padding():
    for method in ("center", "ljust", "rjust"):
        target = bytearray(b"x")
        assert getattr(target, method)(3, target) == b"xxx"

        class Width:
            def __index__(self):
                target.__init__(b"y")
                return 3

        assert getattr(target, method)(Width(), target) == b"yyy"


check_comparison()
check_search()
check_affixes()
check_translate()
check_padding()
