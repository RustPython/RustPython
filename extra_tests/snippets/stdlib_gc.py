"""The cycle collector has to walk the internal fields of containers and
iterators.

Every type below is built into the cycle

    node -> node.__dict__ -> wrapper -> container -> node

so the only path back to `node` runs through a field of the wrapper. A type
that reports nothing while being traversed, or reports the objects it iterates
instead of the iterator it holds, leaves its own reference unaccounted for: the
cycle is then classified as reachable and `node` is never freed.
"""

import gc
import itertools
import weakref
from collections import defaultdict, deque


class Node:
    pass


def collects(wrap):
    """Report whether the collector breaks the cycle built around wrap()."""

    def build():
        container = []
        node = Node()
        container.append(node)
        node.held = wrap(container)
        return weakref.ref(node)

    gc.collect()
    ref = build()
    gc.collect()
    return ref() is None


# containers keeping their items in a field of their own
assert collects(deque)
assert collects(lambda c: defaultdict(int, {"k": c}))
assert collects(lambda c: classmethod(lambda cls: c))

# iterators: the wrapper holds an iterator, and that iterator holds the
# container
assert collects(iter)
assert collects(lambda c: map(str, c))
assert collects(lambda c: filter(None, c))
assert collects(lambda c: zip(c))
assert collects(enumerate)
assert collects(reversed)
assert collects(itertools.chain)
assert collects(itertools.cycle)
assert collects(lambda c: itertools.islice(c, 5))
assert collects(itertools.groupby)
assert collects(itertools.accumulate)
assert collects(lambda c: itertools.starmap(str, c))
assert collects(lambda c: itertools.takewhile(bool, c))
assert collects(lambda c: itertools.dropwhile(bool, c))
assert collects(lambda c: itertools.filterfalse(None, c))
assert collects(lambda c: itertools.compress(c, [1]))
assert collects(lambda c: itertools.product(c))
assert collects(lambda c: itertools.combinations(c, 1))
# tee holds its buffer through a second object, which has to be walked too
assert collects(lambda c: itertools.tee(c)[0])


# A view keeps the object it looks at in a field of its own, and the wrapper a
# `__buffer__` produces keeps the exporter the same way.
class Buf(bytearray):
    pass


def collects_view(wrap):
    """Report whether the collector breaks a cycle that runs through a view."""

    def build():
        container = Buf(b"abc")
        node = Node()
        container.node = node
        node.held = wrap(container)
        return weakref.ref(node)

    gc.collect()
    ref = build()
    gc.collect()
    return ref() is None


assert collects_view(memoryview)
assert collects_view(lambda c: memoryview(c)[1:])
assert collects_view(lambda c: memoryview(c).cast("B"))
assert collects_view(lambda c: memoryview(memoryview(c)))


class Exporter:
    def __buffer__(self, flags):
        return memoryview(b"abcdef")


def collects_exporter():
    def build():
        exporter = Exporter()
        node = Node()
        exporter.node = node
        node.held = memoryview(exporter)
        return weakref.ref(node)

    gc.collect()
    ref = build()
    gc.collect()
    return ref() is None


assert collects_exporter()


def collects_function_annotate():
    from dataclasses import dataclass

    def create():
        @dataclass
        class Example:
            value: int

        return weakref.ref(Example), weakref.ref(Example.__init__)

    class_ref, init_ref = create()
    for _ in range(3):
        gc.collect()

    return class_ref() is None and init_ref() is None


assert collects_function_annotate()


def full_collection_promotes_survivors():
    enabled = gc.isenabled()
    gc.disable()
    try:
        older = []
        gc.collect(0)
        younger = []
        gc.collect(2)
        for value in (older, younger):
            assert any(obj is value for obj in gc.get_objects(2))
            assert not any(obj is value for obj in gc.get_objects(0))
            assert not any(obj is value for obj in gc.get_objects(1))
    finally:
        if enabled:
            gc.enable()


full_collection_promotes_survivors()


def retained_cycles_are_promoted():
    enabled = gc.isenabled()
    debug = gc.get_debug()
    garbage_length = len(gc.garbage)
    retained = []

    class Resurrected:
        def __del__(self):
            retained.append(self)

    gc.disable()
    try:
        for generation in range(3):
            target = min(generation + 1, 2)
            gc.set_debug(0)
            obj = Resurrected()
            obj.cycle = obj
            del obj
            gc.collect(generation)
            assert len(retained) == 1
            assert any(obj is retained[0] for obj in gc.get_objects(target))
            retained[0].cycle = None
            retained.clear()

            gc.set_debug(gc.DEBUG_SAVEALL)
            cycle = []
            cycle.append(cycle)
            identity = id(cycle)
            del cycle
            gc.collect(generation)
            saved = next(obj for obj in gc.garbage if id(obj) == identity)
            assert any(obj is saved for obj in gc.get_objects(target))
            saved.clear()
            del gc.garbage[garbage_length:]
    finally:
        gc.set_debug(debug)
        del gc.garbage[garbage_length:]
        if enabled:
            gc.enable()


retained_cycles_are_promoted()


def check_introspection_audit():
    import sys
    import unittest

    test = unittest.TestCase()
    events = []
    active = True
    deny = False

    class Denied(Exception):
        pass

    def hook(event, args):
        if active and event in (
            "gc.get_objects",
            "gc.get_referrers",
            "gc.get_referents",
        ):
            events.append((event, args))
            if deny:
                raise Denied(event)

    sys.addaudithook(hook)
    try:
        gc.get_objects()
        assert events == [("gc.get_objects", (-1,))]
        for generation in (None, -1, 0, 1, 2):
            events.clear()
            gc.get_objects(generation)
            assert events == [
                ("gc.get_objects", (-1 if generation is None else generation,))
            ]

        for generation, message in (
            (-2, "generation parameter cannot be negative"),
            (
                3,
                "generation parameter must be less than the number of available generations (3)",
            ),
            (
                sys.maxsize,
                "generation parameter must be less than the number of available generations (3)",
            ),
        ):
            events.clear()
            with test.assertRaises(ValueError) as error:
                gc.get_objects(generation)
            assert str(error.exception) == message
            assert events == [("gc.get_objects", (generation,))]

        class Index:
            calls = 0

            def __index__(self):
                self.calls += 1
                return -1

        index = Index()
        events.clear()
        gc.get_objects(index)
        assert index.calls == 1
        assert events == [("gc.get_objects", (-1,))]

        class BadIndex:
            def __index__(self):
                raise Denied("index conversion")

        events.clear()
        with test.assertRaises(Denied):
            gc.get_objects(BadIndex())
        assert not events

        class HugeIndex:
            def __index__(self):
                return sys.maxsize + 1

        class HugeInt(int):
            pass

        for value in (sys.maxsize + 1, HugeIndex(), HugeInt(sys.maxsize + 1)):
            events.clear()
            with test.assertRaises(OverflowError) as error:
                gc.get_objects(value)
            assert str(error.exception) == (
                f"cannot fit '{type(value).__name__}' into an index-sized integer"
            )
            assert not events

        events.clear()
        with test.assertRaises(TypeError) as error:
            gc.get_objects(1.0)
        assert str(error.exception) == "argument should be integer or None, not 'float'"
        assert not events

        import datetime

        with test.assertRaises(TypeError) as error:
            gc.get_objects(datetime.date(2000, 1, 1))
        assert str(error.exception) == (
            "argument should be integer or None, not 'datetime.date'"
        )
        name = "a" + "é" * 110
        truncated_name = name.encode()[:200].decode(errors="ignore")
        with test.assertRaises(TypeError) as error:
            gc.get_objects(type(name, (), {})())
        assert str(error.exception) == (
            f"argument should be integer or None, not '{truncated_name}'"
        )
        with test.assertRaises(OverflowError) as error:
            gc.get_objects(
                type(name, (), {"__index__": lambda self: sys.maxsize + 1})()
            )
        assert str(error.exception) == (
            f"cannot fit '{truncated_name}' into an index-sized integer"
        )
        assert not events

        obj = object()
        container = [obj]
        events.clear()
        referrers = gc.get_referrers(obj)
        assert any(value is container for value in referrers)
        assert events[0][0] == "gc.get_referrers"
        arguments = events[0][1][0]
        assert arguments == (obj,)
        assert all(value is not arguments for value in referrers)
        events.clear()
        assert gc.get_referents(container) == [obj]
        assert events == [("gc.get_referents", ((container,),))]

        deny = True
        for function, argument in (
            (gc.get_objects, -2),
            (gc.get_referrers, obj),
            (gc.get_referents, container),
        ):
            with test.assertRaises(Denied):
                function(argument)
    finally:
        active = False
        events.clear()


check_introspection_audit()


def check_frozen_introspection():
    enabled = gc.isenabled()
    gc.disable()
    target = []
    frozen_referrer = [target]

    def contains(objects, target):
        return any(obj is target for obj in objects)

    try:
        gc.freeze()
        fresh_referrer = [target]
        assert gc.get_freeze_count() > 0
        assert gc.is_tracked(frozen_referrer)
        assert not contains(gc.get_objects(), frozen_referrer)
        assert not contains(gc.get_objects(None), frozen_referrer)
        assert not contains(gc.get_objects(-1), frozen_referrer)
        for generation in range(3):
            assert not contains(gc.get_objects(generation), frozen_referrer)
        referrers = gc.get_referrers(target)
        assert not contains(referrers, frozen_referrer)
        assert contains(referrers, fresh_referrer)
        # Freezing changes the scanned generations, not the object's edges.
        assert gc.get_referents(frozen_referrer) == [target]

        gc.unfreeze()
        assert contains(gc.get_objects(2), frozen_referrer)
        assert contains(gc.get_referrers(target), frozen_referrer)
    finally:
        gc.unfreeze()
        if enabled:
            gc.enable()


check_frozen_introspection()


def check_gc_configuration():
    import struct
    import unittest

    test = unittest.TestCase()
    old_thresholds = gc.get_threshold()
    old_debug = gc.get_debug()
    enabled = gc.isenabled()
    gc.disable()
    try:
        for flags in (0, 1 << 30, -128):
            gc.set_debug(flags)
            assert gc.get_debug() == flags
        gc.set_debug(0)
        for values in ((-1, -2, -3), (0, 10, 20), (2**31 - 1, -(2**31), 0)):
            gc.set_threshold(*values)
            assert gc.get_threshold() == values
        gc.set_threshold(7)
        assert gc.get_threshold() == (7, -(2**31), 0)
        gc.set_threshold(8, 9)
        assert gc.get_threshold() == (8, 9, 0)

        calls = []

        class Index:
            def __init__(self, value):
                self.value = value

            def __index__(self):
                calls.append(self.value)
                return self.value

        gc.set_threshold(Index(12), Index(13), Index(14))
        assert calls == [12, 13, 14]
        assert gc.get_threshold() == (12, 13, 14)
        calls.clear()
        for args in ((), (Index(1), Index(2), Index(3), Index(4))):
            with test.assertRaises(TypeError) as error:
                gc.set_threshold(*args)
            assert str(error.exception) == "gc.set_threshold requires 1 to 3 arguments"
        with test.assertRaises(TypeError) as error:
            gc.set_threshold(Index(1), threshold1=Index(2))
        assert str(error.exception) == "set_threshold() takes no keyword arguments"
        assert calls == []

        # A failing later conversion must not partially update the settings.
        with test.assertRaises(TypeError):
            gc.set_threshold(Index(21), object())
        assert calls == [21]
        assert gc.get_threshold() == (12, 13, 14)
        long_is_int = struct.calcsize("l") == 4
        for value, message in (
            (
                2**31,
                "Python int too large to convert to C long"
                if long_is_int
                else "signed integer is greater than maximum",
            ),
            (
                -(2**31) - 1,
                "Python int too large to convert to C long"
                if long_is_int
                else "signed integer is less than minimum",
            ),
            (2**100, "Python int too large to convert to C long"),
            (-(2**100), "Python int too large to convert to C long"),
        ):
            with test.assertRaises(OverflowError) as error:
                gc.set_threshold(value)
            assert str(error.exception) == message
        for function in (gc.collect, gc.set_debug):
            for value in (2**31, -(2**31) - 1, 2**100):
                with test.assertRaises(OverflowError) as error:
                    function(value)
                assert (
                    str(error.exception) == "Python int too large to convert to C int"
                )
    finally:
        gc.set_debug(old_debug)
        gc.set_threshold(*old_thresholds)
        if enabled:
            gc.enable()


check_gc_configuration()


def check_gc_callback_semantics():
    import sys

    enabled = gc.isenabled()
    saved_callbacks = gc.callbacks[:]
    saved_hook = sys.unraisablehook
    gc.disable()
    gc.callbacks.clear()
    try:
        calls = []

        def recursive(phase, info):
            calls.append(phase)
            calls.append(gc.collect())

        gc.callbacks.append(recursive)
        gc.collect()
        assert calls == ["start", 0, "stop", 0]

        def second(phase, info):
            calls.append(("second", phase))

        def appended(phase, info):
            calls.append(("appended", phase))

        def first(phase, info):
            calls.append(("first", phase))
            if phase == "start":
                if mutation == "remove":
                    gc.callbacks.remove(first)
                elif mutation == "append":
                    gc.callbacks.append(appended)
                else:
                    gc.callbacks.clear()

        for mutation, expected in (
            ("remove", [("first", "start"), ("second", "stop")]),
            (
                "append",
                [
                    ("first", "start"),
                    ("second", "start"),
                    ("appended", "start"),
                    ("first", "stop"),
                    ("second", "stop"),
                    ("appended", "stop"),
                ],
            ),
            ("clear", [("first", "start")]),
        ):
            calls.clear()
            gc.callbacks[:] = [first, second]
            gc.collect()
            assert calls == expected, calls

        errors = []
        error = ValueError("callback failed")

        def unraisable(args):
            assert gc.collect() == 0
            errors.append((args.exc_value, args.err_msg, args.object))

        sys.unraisablehook = unraisable

        def failing(phase, info):
            raise error

        class BadRepr:
            def __call__(self, phase, info):
                raise error

            def __repr__(self):
                raise RuntimeError("repr failed")

        for callback, message in (
            (failing, "Exception ignored while calling GC callback " + repr(failing)),
            (BadRepr(), None),
        ):
            errors.clear()
            calls.clear()
            gc.callbacks[:] = [callback, second]
            gc.collect()
            assert calls == [("second", "start"), ("second", "stop")]
            assert errors == [(error, message, None), (error, message, None)]
    finally:
        gc.callbacks[:] = saved_callbacks
        sys.unraisablehook = saved_hook
        if enabled:
            gc.enable()


check_gc_callback_semantics()

print("ok")
