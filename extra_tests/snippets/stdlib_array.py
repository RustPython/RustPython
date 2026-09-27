import sys
from array import array
from pickle import dumps, loads

from testutils import assert_raises

a1 = array("b", [0, 1, 2, 3])

assert a1.tobytes() == b"\x00\x01\x02\x03"
assert a1[2] == 2

assert list(a1) == [0, 1, 2, 3]

a1.reverse()
assert a1 == array("B", [3, 2, 1, 0])

a1.extend([4, 5, 6, 7])

assert a1 == array("h", [3, 2, 1, 0, 4, 5, 6, 7])

# eq, ne
a = array("b", [0, 1, 2, 3])
b = a
assert a.__ne__(b) is False
b = array("B", [3, 2, 1, 0])
assert a.__ne__(b) is True


def test_float_with_integer_input():
    f = array("f", [0, 1, 2.0, 3.0])
    f.append(4)
    f.insert(0, -1)
    assert f.count(4) == 1
    f.remove(1)
    assert f.index(0) == 1
    f[0] = -2
    assert f == array("f", [-2, 0, 2, 3, 4])


test_float_with_integer_input()

# slice assignment step overflow behaviour test
T = "I"
a = array(T, range(10))
b = array(T, [100])
a[::9999999999] = b
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 9])
a[::-9999999999] = b
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 100])
c = array(T)
a[0:0:9999999999] = c
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 100])
a[0:0:-9999999999] = c
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 100])
del a[::9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8, 100])
del a[::-9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8])
del a[0:0:9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8])
del a[0:0:-9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8])


def test_float_with_nan():
    f = float("nan")
    a = array("f")
    a.append(f)
    assert not (a == a)
    assert a != a
    assert not (a < a)
    assert not (a <= a)
    assert not (a > a)
    assert not (a >= a)


test_float_with_nan()


def test_different_type_cmp():
    a = array("i", [-1, -2, -3, -4])
    b = array("I", [1, 2, 3, 4])
    c = array("f", [1, 2, 3, 4])
    assert a < b
    assert b > a
    assert b == c
    assert a < c
    assert c > a


test_different_type_cmp()


def test_array_frombytes():
    a = array("b", [-1, -2])
    b = bytearray(a.tobytes())
    c = array("b", b)
    assert a == c


test_array_frombytes()

# test that indexing on an empty array doesn't panic
a = array("b")
with assert_raises(IndexError):
    a[0]
with assert_raises(IndexError):
    a[0] = 42
with assert_raises(IndexError):
    del a[42]

test_str = "🌉abc🌐def🌉🌐"
u = array("u", test_str)
# skip as 2 bytes character environment with CPython is failing the test
if u.itemsize >= 4:
    assert u.__reduce_ex__(1)[1][1] == list(test_str)
    assert loads(dumps(u, 1)) == loads(dumps(u, 3))

# test array name
a = array("b", [])
assert str(a.__class__.__name__) == "array"
# test arrayiterator name
i = iter(a)
assert str(i.__class__.__name__) == "arrayiterator"

# teset array.__contains__
a = array("B", [0])
assert a.__contains__(0)
assert not a.__contains__(1)


class _ReenteringWriter:
    def __init__(self, arr):
        self.arr = arr
        self.reentered = False

    def write(self, chunk):
        if not self.reentered:
            self.reentered = True
            self.arr.append(0)
        return len(chunk)


arr = array("b", range(128))
arr.tofile(_ReenteringWriter(arr))
assert len(arr) == 129


def test_setitem_reentrant():
    # Converting the value runs Python, which can reach the array, so the
    # array is not locked while it happens.
    a = array("i", [1, 2, 3])

    class Index:
        def __index__(self):
            a[1] = 9
            return 7

    a[0] = Index()
    assert a == array("i", [7, 9, 3]), a


test_setitem_reentrant()


def test_frombytes_of_itself():
    # Resizing is refused while a buffer is exported, before any lock is taken.
    # The typecode is "b" so the view's items are bytes and the resize is what
    # the call is refused for.
    a = array("b", [1, 2, 3])
    m = memoryview(a)
    with assert_raises(BufferError):
        a.frombytes(m)
    del m

    # A view of wider items is not a source of bytes at all.
    wide = array("i", [1, 2, 3])
    with assert_raises(TypeError):
        wide.frombytes(memoryview(wide))


# Repeating an empty array by a huge count returns at once
empty = array("i")
assert empty * sys.maxsize == array("i")
empty *= sys.maxsize
assert empty == array("i")


def test_array_search_uses_python_equality():
    for typecode, stored, needle, matches in (
        ("i", 1, 1.0, True),
        ("f", 0.1, 0.1, False),
        ("Q", 2**53 + 1, float(2**53), False),
        ("b", 1, 257, False),
        ("d", float("nan"), float("nan"), False),
    ):
        values = array(typecode, [stored])
        assert (needle in values) is matches
        assert values.count(needle) == int(matches)
        if matches:
            assert values.index(needle) == 0
            values.remove(needle)
            assert not values
        else:
            with assert_raises(ValueError):
                values.index(needle)
            with assert_raises(ValueError):
                values.remove(needle)
            assert len(values) == 1

    class DifferentInt(int):
        def __eq__(self, other):
            return False

    values = array("i", [1])
    assert values.count(DifferentInt(1)) == 0
    assert DifferentInt(1) not in values

    invalid = array("w")
    invalid.frombytes((0x110000).to_bytes(4, sys.byteorder))
    invalid.append("x")
    for operation, _ in _SEARCH_OPERATIONS:
        with assert_raises(ValueError):
            operation(invalid, "x")
    invalid.reverse()
    assert invalid.index("x") == 0
    assert "x" in invalid
    with assert_raises(ValueError):
        invalid.count("x")


_SEARCH_OPERATIONS = (
    (lambda values, needle: values.count(needle), 1),
    (lambda values, needle: values.index(needle), 0),
    (lambda values, needle: values.remove(needle), None),
    (lambda values, needle: needle in values, True),
)


def test_array_search_callbacks():
    class ComparisonError:
        def __eq__(self, other):
            raise RuntimeError("comparison failed")

    for operation, expected in _SEARCH_OPERATIONS:
        values = array("i", [1])
        with assert_raises(RuntimeError):
            operation(values, ComparisonError())

        views = []

        class ClearOnComparison:
            def __eq__(self, other):
                values.clear()
                views.append(memoryview(values))
                return True

        assert operation(values, ClearOnComparison()) == expected
        assert not values
        views.pop().release()


def test_array_search_sees_appended_items():
    for stop in (None, 10):
        values = array("i", [1])

        class AppendOnComparison:
            def __eq__(self, other):
                if other == 1:
                    values.append(2)
                return other == 2

        needle = AppendOnComparison()
        result = values.index(needle) if stop is None else values.index(needle, 0, stop)
        assert result == 1

    values = array("i", [1])

    class Start:
        def __index__(self):
            values.append(2)
            return -1

    assert values.index(2, Start()) == 1


_ARRAY_INSERTIONS = (
    lambda values, value: values.append(value),
    lambda values, value: values.insert(0, value),
)


def test_array_insert_converts_without_lock():
    for operation in _ARRAY_INSERTIONS:
        values = array("i", [0])
        view = memoryview(values)

        class ReleaseExport:
            def __index__(self):
                view.release()
                values[0] = 3
                return 7

        operation(values, ReleaseExport())
        assert sorted(values) == [3, 7]

        views = []

        class CreateExport:
            def __index__(self):
                views.append(memoryview(values))
                return 9

        with assert_raises(BufferError):
            operation(values, CreateExport())
        assert sorted(values) == [3, 7]
        views.pop().release()

        class BadConversion:
            def __index__(self):
                raise RuntimeError("conversion failed")

        with memoryview(values):
            with assert_raises(RuntimeError):
                operation(values, BadConversion())


def test_array_extend_callbacks_and_partial_progress():
    values = array("i", [0])

    def source():
        values[0] = 1
        yield 2
        assert list(values) == [1, 2]
        values.append(3)
        yield 4
        raise RuntimeError("iterator failed")

    with assert_raises(RuntimeError):
        values.extend(source())
    assert list(values) == [1, 2, 3, 4]


test_array_search_uses_python_equality()
test_array_search_callbacks()
test_array_search_sees_appended_items()
test_array_insert_converts_without_lock()
test_array_extend_callbacks_and_partial_progress()
