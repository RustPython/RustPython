"""
Tests for the pyo3 module, the bridge from RustPython to an embedded CPython.

Requires the `pyo3` feature:
    cargo build --release --features pyo3
    ./target/release/rustpython extra_tests/test_module_pyo3.py
"""

import array
import collections.abc
import copy
import struct
import sys
import time

import pyo3

cpython = pyo3.import_module("builtins")
_decimal = pyo3.import_module("_decimal")
Decimal = _decimal.Decimal


# pyo3.wraps runs the whole function in CPython.


@pyo3.wraps
def max_prec():
    import _decimal

    return _decimal.MAX_PREC


@pyo3.wraps
def describe(*args, **kwargs):
    import sys

    return (sys.implementation.name, args, sorted(kwargs.items()))


assert max_prec() == 999999999999999999
assert describe(1, "a", key=[2]) == ("cpython", (1, "a"), [("key", [2])])
assert max_prec.__name__ == "max_prec"
assert max_prec.__wrapped__.__name__ == "max_prec"


# Immutable built-in values cross by value.

assert cpython.len("héllo") == 5
assert cpython.pow(2, 100) == 2**100
assert cpython.pow(-3, 101) == (-3) ** 101
assert cpython.abs(-1.5) == 1.5
assert cpython.complex(1, 2) == 1 + 2j
assert cpython.bytes([104, 105]) == b"hi"
assert cpython.tuple("ab") == ("a", "b")
assert cpython.str("\udc80") == "\udc80"  # lone surrogates survive
assert cpython.isinstance(None, cpython.type(None))


# CPython objects are proxies; CPython classes are wrapper classes.

d = Decimal("1.1")
assert isinstance(d, pyo3.ref)
assert isinstance(d, Decimal)
assert d.__class__ is Decimal
assert type(d) is Decimal
assert type(Decimal("1.5")) is Decimal
assert not isinstance(Decimal("1"), collections.abc.Iterable)
assert copy.copy(Decimal("1.5")) == Decimal("1.5")
assert Decimal.__name__ == "Decimal"
assert str(d) == "1.1"
assert repr(d) == "Decimal('1.1')"
assert f"{d:>6}" == "   1.1"
assert hash(d) == cpython.hash(d)

# Classes are cached: one wrapper per CPython class.
assert _decimal.Decimal is Decimal
assert type(Decimal) is pyo3.pyo3_type

# CPython's built-in classes are RustPython's.
assert cpython.int is int
assert cpython.type is type
assert cpython.ValueError is ValueError


# Operators

d2 = Decimal("2.2")
assert str(d + d2) == "3.3"
assert str(d + 1) == "2.1"
assert str(1 + d) == "2.1"  # reflected: int.__add__ returns NotImplemented
assert str(d * 2) == "2.2"
assert str(-d) == "-1.1"
assert str(abs(-d)) == "1.1"
assert str(d2**2) == "4.84"
q, r = divmod(Decimal(7), 2)
assert (str(q), str(r)) == ("3", "1")
assert d < d2 and d <= d and d2 > d and d == Decimal("1.1") and d != d2
assert int(Decimal("3.7")) == 3
assert float(Decimal("0.5")) == 0.5
assert round(Decimal("2.675"), 1) == Decimal("2.7")

acc = Decimal(0)
acc += 5
assert str(acc) == "5"


# Exceptions become the nearest built-in exception class.

try:
    Decimal(1) / 0
except ZeroDivisionError as e:  # decimal.DivisionByZero
    assert any("DivisionByZero" in note for note in e.__notes__), e.__notes__
else:
    raise AssertionError("expected ZeroDivisionError")

try:
    pyo3.import_module("no_such_module_for_pyo3")
except ModuleNotFoundError as e:
    assert e.name == "no_such_module_for_pyo3"
else:
    raise AssertionError("expected ModuleNotFoundError")


try:
    cpython.getattr(Decimal(1), "no_such_attribute")
except AttributeError as e:
    assert e.name == "no_such_attribute"
else:
    raise AssertionError("expected AttributeError")

cyclic = []
cyclic.append(cyclic)
try:
    cpython.len(cyclic)
except RecursionError:
    pass
else:
    raise AssertionError("expected RecursionError")


# RustPython callables are reverse proxies; exceptions keep their identity.

_functools = pyo3.import_module("_functools")
assert _functools.reduce(lambda a, b: a * b, [1, 2, 3, 4]) == 24
assert list(cpython.sorted([3, 1, 2], key=lambda x: -x)) == [3, 2, 1]
assert list(cpython.map(str.upper, ["a", "b"])) == ["A", "B"]


class Custom(Exception):
    pass


error = Custom("from RustPython")


def fail(a, b):
    raise error


try:
    _functools.reduce(fail, [1, 2])
except Custom as e:
    assert e is error
else:
    raise AssertionError("expected Custom")


# RustPython code called from CPython runs with the GIL released.


@pyo3.wraps
def start_counter():
    import threading

    state = {"count": 0, "stop": False}

    def count():
        while not state["stop"]:
            state["count"] += 1

    thread = threading.Thread(target=count)
    thread.start()
    return state, thread


@pyo3.wraps
def call(f):
    return f()


def counted_while_sleeping():
    before = state["count"]
    time.sleep(0.2)
    return state["count"] - before


state, thread = start_counter()
try:
    assert call(counted_while_sleeping) > 0
finally:
    state["stop"] = True
    thread.join()


# Containers

_collections = pyo3.import_module("_collections")
dq = _collections.deque([1, 2, 3])
assert len(dq) == 3
assert dq[0] == 1 and dq[-1] == 3
assert 2 in dq and 5 not in dq
dq.append(4)
dq[0] = 10
del dq[1]
assert list(dq) == [10, 3, 4]
assert list(reversed(dq)) == [4, 3, 10]
assert bool(dq) and not bool(_collections.deque())
assert isinstance(_collections.deque(), collections.abc.Iterable)

od = _collections.OrderedDict()
od["b"] = 1
od["a"] = 2
assert list(od) == ["b", "a"]
assert od["a"] == 2
assert "b" in od

# RustPython lists and dicts are copied into CPython.
items = [1, 2]
assert cpython.len(items) == 2
assert cpython.sorted({"b": 1, "a": 2}) == cpython.sorted(["a", "b"])


# Buffers

# A CPython buffer is shared with RustPython, not copied.
cpython_array = pyo3.import_module("array").array("i", [1, 2, 3])
view = memoryview(cpython_array)
assert (view.format, view.itemsize, view.shape) == ("i", 4, (3,))
assert view.tolist() == [1, 2, 3]
view[0] = 100
assert cpython_array[0] == 100
assert memoryview(cpython.memoryview(cpython_array)[::-1]).tolist() == [3, 2, 100]
assert memoryview(cpython.bytes(b"ab")).readonly

# A RustPython buffer is shared with CPython, not copied.
_struct = pyo3.import_module("_struct")
buf = array.array("b", [0] * 4)
_struct.pack_into("<i", buf, 0, 0x01020304)
assert buf.tolist() == [4, 3, 2, 1]
assert cpython.bytes(buf) == b"\x04\x03\x02\x01"


# Class statements deriving from wrapped classes create CPython classes.


class Mixin:
    def triple(self):
        return self * 3


class MyDecimal(Mixin, Decimal):
    def double(self):
        return self * 2

    def __repr__(self):
        return f"MyDecimal({super().__repr__()})"


m = MyDecimal("1.5")
assert isinstance(m, MyDecimal) and isinstance(m, Decimal) and isinstance(m, Mixin)
assert issubclass(MyDecimal, Decimal) and issubclass(MyDecimal, Mixin)
assert MyDecimal.__mro__[0] is MyDecimal
assert str(m.double()) == "3.0"
assert str(m.triple()) == "4.5"
assert repr(m) == "MyDecimal(Decimal('1.5'))"
assert cpython.repr(m) == repr(m)  # CPython sees the override too
assert cpython.isinstance(m, Decimal)


# Class attributes are forwarded both ways.

MyDecimal.attr = 42
assert MyDecimal.attr == 42
assert m.attr == 42
assert cpython.getattr(MyDecimal, "attr") == 42
del MyDecimal.attr
assert not hasattr(MyDecimal, "attr")
assert not cpython.hasattr(MyDecimal, "attr")

try:
    Decimal.attr = 42
except TypeError:
    pass  # immutable in CPython
else:
    raise AssertionError("expected TypeError")


# ctypes: metaclasses, class statements and callbacks

ctypes = pyo3.import_module("ctypes")
c_int = ctypes.c_int
assert type(ctypes.Structure).__name__ == "PyCStructType"
assert type(ctypes.c_int).__name__ == "PyCSimpleType"
assert not callable(Decimal("1")) and callable(ctypes.CFUNCTYPE(None))
assert bytes(ctypes.c_double(0.5)) == struct.pack("d", 0.5)
d2 = ctypes.c_double(2.5)
assert type(ctypes.c_double.from_address(ctypes.addressof(d2))) is ctypes.c_double
assert ctypes.sizeof(c_int) == 4
arr = (c_int * 4)(1, 2, 3, 4)
assert list(arr) == [1, 2, 3, 4] and len(arr) == 4 and arr[2] == 3


class Point(ctypes.Structure):
    _fields_ = [("x", c_int), ("y", c_int)]

    def norm1(self):
        return abs(self.x) + abs(self.y)


p = Point(3, -4)
assert (p.x, p.y, p.norm1(), ctypes.sizeof(Point)) == (3, -4, 7, 8)
p.x = 10
assert p.x == 10


class Rect(ctypes.Structure):
    pass


Rect._fields_ = [("a", Point), ("b", Point)]  # processed by CPython's metaclass
assert Rect(Point(1, 2), Point(3, 4)).b.y == 4
assert ctypes.sizeof(Rect) == 16

n = c_int(5)
ctypes.pointer(n).contents.value = 7
assert n.value == 7

CMPFUNC = ctypes.CFUNCTYPE(c_int, ctypes.POINTER(c_int), ctypes.POINTER(c_int))
values = (c_int * 5)(5, 1, 4, 2, 3)
libc = ctypes.cdll.msvcrt if sys.platform == "win32" else ctypes.CDLL(None)
libc.qsort(values, len(values), ctypes.sizeof(c_int), CMPFUNC(lambda a, b: a[0] - b[0]))
assert list(values) == [1, 2, 3, 4, 5]

# ctypes buffers leave out strides, and scalars have zero dimensions.
assert memoryview(values).tolist() == [1, 2, 3, 4, 5]
d = ctypes.c_double(1.5)
view = memoryview(d)
assert (view.ndim, view.shape, view.format) == (0, (), "<d")
d.value = 2.5
assert bytes(view) == struct.pack("<d", 2.5)


class EmptyStruct(ctypes.Structure):
    pass


# A resized scalar spans its whole buffer, not just its itemsize.
empty = EmptyStruct()
ctypes.resize(empty, 5)
memoryview(empty).cast("B")[:] = b"hello"
assert bytes(memoryview(empty)) == b"hello"


# numpy, when available

try:
    np = pyo3.import_module("numpy")
except ImportError:
    np = None

if np is not None:
    a = np.array([1, 2, 3])
    assert isinstance(a, pyo3.ref)
    assert (a * 2).tolist() == [2, 4, 6]
    assert (a + a).tolist() == [2, 4, 6]
    assert a.sum() == 6
    assert len(a) == 3 and a[1] == 2 and list(a[1:]) == [2, 3]
    assert (a == np.array([1, 0, 3])).tolist() == [True, False, True]
    assert memoryview(a).tolist() == [1, 2, 3]
    m = np.array([[1, 2], [3, 4]])
    assert m.T.tolist() == [[1, 3], [2, 4]]
    assert (m @ m).tolist() == [[7, 10], [15, 22]]

print("OK")
