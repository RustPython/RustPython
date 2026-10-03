"""CPython 3.15 variable-base slots; imports only built-in modules.

This intentionally runs each case after a failure so an older RustPython
binary supplies useful differential evidence without an ahead-of-runtime Lib.
"""

import _weakref
import gc

failures = []
count = 0


def check(name, fn):
    global count
    count += 1
    try:
        fn()
    except Exception as exc:
        failures.append(name)
        print("FAIL", name, type(exc).__name__, str(exc))
    else:
        print("PASS", name)


def raises(kind, fn, prefix=None):
    try:
        fn()
    except kind as exc:
        if prefix is not None:
            assert str(exc).startswith(prefix), str(exc)
    else:
        raise AssertionError("expected " + kind.__name__)


def instance(cls, base):
    return (
        cls((1, 2, 3))
        if base is tuple
        else cls(b"payload")
        if base is bytes
        else cls(12345)
    )


def slot_matrix(base, slots):
    cls = type("Variable", (base,), {"__slots__": slots})
    obj = instance(cls, base)
    expected = (1, 2, 3) if base is tuple else b"payload" if base is bytes else 12345
    assert obj == expected
    assert len(obj) == len(expected) if base is not int else obj + 5 == 12350
    assert hasattr(obj, "__dict__") == ("__dict__" in slots)
    if "__dict__" in slots:
        obj.dynamic = "dict value"
        assert obj.__dict__ == {"dynamic": "dict value"}
    else:
        raises(AttributeError, lambda: setattr(obj, "dynamic", 1))
    if "__weakref__" in slots:
        ref = _weakref.ref(obj)
        assert ref() is obj
        del obj
        assert ref() is None
    else:
        raises(TypeError, lambda: _weakref.ref(obj))


for base in (tuple, bytes, int):
    for slots in ((), ("__dict__",), ("__weakref__",), ("__dict__", "__weakref__")):
        check(
            base.__name__ + "_" + repr(slots), lambda b=base, s=slots: slot_matrix(b, s)
        )


def implicit_weakrefs():
    for base in (tuple, bytes, int):
        cls = type("Implicit", (base,), {})
        obj = instance(cls, base)
        obj.value = 10
        assert obj.__dict__ == {"value": 10}
        raises(TypeError, lambda: _weakref.ref(obj))


check("implicit_weakrefs_unchanged", implicit_weakrefs)


def rejected_named_slots():
    for base in (bytes, int):
        for slots in (
            ("x",),
            ("x", "__dict__"),
            ("x", "__weakref__"),
            ("x", "__dict__", "__weakref__"),
        ):
            raises(
                TypeError,
                lambda: type("Rejected", (base,), {"__slots__": slots}),
                "arbitrary __slots__ not supported for subtype of",
            )

    raises(
        TypeError,
        lambda: type("Rejected", (type,), {"__slots__": ("x",)}),
        "arbitrary __slots__ not supported for subtype of",
    )


check("non_tuple_named_slots_rejected", rejected_named_slots)


def tuples():
    class T(tuple):
        __slots__ = ("x", "__dict__", "__weakref__")

    for length in (0, 1, 2, 31, 200):
        payload = tuple(range(length))
        obj = T(iter(payload))
        assert tuple(obj) == payload and obj[:] == payload
        assert list(obj) == list(payload) and hash(obj) == hash(payload)
        raises(AttributeError, lambda: obj.x)
        obj.x = payload
        obj.dynamic = "ok"
        assert obj.x is payload
        ref = _weakref.ref(obj)
        assert ref() is obj
        del obj.x
        raises(AttributeError, lambda: obj.x)
        del obj
        assert ref() is None


check("tuple_payloads_and_mixed_slots", tuples)


def inheritance():
    class T(tuple):
        __slots__ = ("x",)

    class U(T):
        __slots__ = ("y", "__weakref__")

    class V(U):
        __slots__ = ("z", "__dict__")

    class W(V):
        __slots__ = ()

    obj = W((7, 8))
    obj.x, obj.y, obj.z, obj.dynamic = "x", "y", "z", "dynamic"
    assert (obj.x, obj.y, obj.z, obj.dynamic) == ("x", "y", "z", "dynamic")
    assert T.x.__get__(obj) == "x" and U.y.__get__(obj) == "y"
    assert tuple(obj) == (7, 8)
    ref = _weakref.ref(obj)
    del obj
    assert ref() is None
    for cls, slot in ((U, "__weakref__"), (V, "__dict__")):
        raises(TypeError, lambda: type("Duplicate", (cls,), {"__slots__": (slot,)}))


check("inherited_prefix_slots_and_special_slots", inheritance)


def duplicates():
    for slot in ("__dict__", "__weakref__"):
        raises(
            TypeError,
            lambda: type("Duplicate", (tuple,), {"__slots__": (slot, slot)}),
            slot + " slot disallowed:",
        )
    raises(
        TypeError,
        lambda: type("Invalid", (tuple,), {"__slots__": ("not valid",)}),
        "__slots__ must be identifiers",
    )
    raises(
        ValueError,
        lambda: type("Conflict", (tuple,), {"__slots__": ("x",), "x": 1}),
        "'x' in __slots__ conflicts with class variable",
    )


check("invalid_and_duplicate_slots", duplicates)


def multiple_inheritance():
    class T(tuple):
        __slots__ = ("x",)

    class U(tuple):
        __slots__ = ("y",)

    class S:
        __slots__ = ("z",)

    for bases in ((T, U), (U, T), (T, S), (S, T)):
        raises(
            TypeError,
            lambda: type("Conflict", bases, {"__slots__": ()}),
            "multiple bases have instance ",
        )

    class Empty:
        __slots__ = ()

    class Mix(T, Empty):
        __slots__ = ("y",)

    obj = Mix((1, 2))
    obj.x, obj.y = 1, 2
    assert (obj.x, obj.y, tuple(obj)) == (1, 2, (1, 2))

    class Left(T):
        __slots__ = ()

    class Right(T):
        __slots__ = ()

    class Diamond(Left, Right):
        __slots__ = ("z",)

    obj = Diamond((3, 4))
    obj.x, obj.z = "root", "leaf"
    assert (obj.x, obj.z, tuple(obj)) == ("root", "leaf", (3, 4))


check("multiple_inheritance_layout_conflicts", multiple_inheritance)


def assignment():
    class A(tuple):
        __slots__ = ("x", "y", "__dict__", "__weakref__")

    class B(tuple):
        __slots__ = ("__weakref__", "y", "__dict__", "x")

    obj = A((11, 22, 33))
    obj.x, obj.y, obj.dynamic = "x", "y", "dict"
    ref = _weakref.ref(obj)
    obj.__class__ = B
    assert type(obj) is B and ref() is obj
    assert (obj.x, obj.y, obj.dynamic, tuple(obj)) == ("x", "y", "dict", (11, 22, 33))
    for slots in (
        ("x",),
        ("x", "z", "__dict__", "__weakref__"),
        ("x", "y", "__weakref__"),
        ("x", "y", "__dict__"),
        ("x", "y", "z", "__dict__", "__weakref__"),
    ):
        bad = type("Bad", (tuple,), {"__slots__": slots})
        raises(
            TypeError, lambda: setattr(obj, "__class__", bad), "__class__ assignment:"
        )
        assert type(obj) is B and obj.x == "x" and obj.y == "y" and ref() is obj
    del obj
    assert ref() is None


check("class_assignment_preserves_payload_and_prefix", assignment)


def base_assignment():
    class A(tuple):
        __slots__ = ("x",)

    class B(tuple):
        __slots__ = ("x",)

    class C(tuple):
        __slots__ = ("y",)

    class Child(A):
        __slots__ = ("z", "__weakref__")

    obj = Child((3, 5))
    obj.x, obj.z = "root", "leaf"
    ref = _weakref.ref(obj)
    Child.__bases__ = (B,)
    assert (obj.x, obj.z, tuple(obj), ref() is obj) == ("root", "leaf", (3, 5), True)
    raises(
        TypeError, lambda: setattr(Child, "__bases__", (C,)), "__bases__ assignment:"
    )
    assert Child.__bases__ == (B,) and obj.x == "root" and obj.z == "leaf"
    del obj
    assert ref() is None


check("bases_assignment_layout_safety", base_assignment)


def cycles():
    class T(tuple):
        __slots__ = ("x", "__dict__", "__weakref__")

    callbacks = []
    refs = []
    for mode in ("slot", "dict", "tuple_element", "cross_prefix_payload"):
        payload = []
        obj = T((payload,))
        refs.append(_weakref.ref(obj, lambda ref: callbacks.append("collected")))
        if mode == "slot":
            obj.x = obj
        elif mode == "dict":
            obj.dynamic = obj
        elif mode == "tuple_element":
            payload.append(obj)
        else:
            obj.x = payload
            payload.append(obj)
        del obj, payload
    for _ in range(3):
        gc.collect()
    assert all(ref() is None for ref in refs)
    assert len(callbacks) == 4


check("slot_dict_tuple_payload_cycles", cycles)

print("RESULT", count - len(failures), "passed", len(failures), "failed")
if failures:
    raise AssertionError(", ".join(failures))
