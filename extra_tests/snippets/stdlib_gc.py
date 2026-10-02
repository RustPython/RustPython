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
import sys
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
assert collects(tuple)
assert collects(lambda c: defaultdict(int, {"k": c}))
assert collects(lambda c: classmethod(lambda cls: c))
assert collects(lambda c: c.append)
assert collects(lambda c: c.__iter__)

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


def automatic_collection_visits_older_generations():
    enabled = gc.isenabled()
    thresholds = gc.get_threshold()
    phases = []

    def callback(phase, info):
        phases.append((phase, info["generation"]))
        # A callback must not recursively start another collection.
        assert gc.collect() == 0

    gc.disable()
    try:
        gc.collect()
        node = Node()
        node.cycle = node
        ref = weakref.ref(node)
        gc.collect(2)
        del node
        gc.callbacks.append(callback)
        gc.set_threshold(50, 1, 1)
        gc.enable()
        held = [[i] for i in range(5000)]
        gc.disable()
        assert ref() is None
        assert len(held) == 5000
        assert {generation for phase, generation in phases} == {0, 1, 2}
        assert len(phases) % 2 == 0
        for start, stop in zip(phases[::2], phases[1::2]):
            assert start[0] == "start" and stop == ("stop", start[1])
    finally:
        gc.callbacks.remove(callback)
        gc.set_threshold(*thresholds)
        if enabled:
            gc.enable()


automatic_collection_visits_older_generations()


def finalizer_can_replace_edges_and_create_weakrefs():
    refs = []
    calls = []

    class Finalized:
        def __del__(self):
            refs.append(weakref.ref(self, lambda ref: calls.append(ref)))
            self.cycle = [self]

    node = Finalized()
    node.cycle = node
    del node
    # The replacement list is new and may be collected in the next cycle.
    gc.collect()
    gc.collect()
    assert len(refs) == 1 and refs[0]() is None
    assert calls == refs


finalizer_can_replace_edges_and_create_weakrefs()


def immutable_tuple_chains_and_subclass_cycles():
    chain = None
    for _ in range(20000):
        chain = (chain,)
    gc.collect()
    # Releasing an untracked immutable chain still needs bounded stack usage.
    del chain

    class Tuple(tuple):
        pass

    obj = Tuple((1,))
    obj.cycle = obj
    node = Node()
    obj.node = node
    ref = weakref.ref(node)
    del node, obj
    gc.collect()
    assert ref() is None


immutable_tuple_chains_and_subclass_cycles()


def native_descriptors_release_their_references():
    if sys.implementation.name == "rustpython":
        # CPython 3.14 also retains this cycle: builtin methods have no tp_clear.
        container = []
        references = sys.getrefcount(container)
        method = container.append
        method.__module__ = method
        del method
        gc.collect()
        assert sys.getrefcount(container) == references

    class Slotted:
        __slots__ = ("value",)

    ref = weakref.ref(Slotted)
    descriptor = Slotted.value
    del Slotted
    gc.collect()
    assert ref() is descriptor.__objclass__
    del descriptor
    gc.collect()
    assert ref() is None


native_descriptors_release_their_references()


def weakref_callbacks_do_not_hide_cycles():
    for factory in (weakref.ref, weakref.proxy):
        marker = Node()
        ref = weakref.ref(marker)
        items = [marker]

        def callback(_, items=items):
            pass

        items.append(factory(int, callback))
        del marker, items, callback
        gc.collect()
        assert ref() is None


weakref_callbacks_do_not_hide_cycles()


def only_reachable_weakrefs_run_callbacks():
    for keep_ref in (False, True):
        events = []
        node = Node()
        roots = [node]
        node.cycle = node

        def callback(_, roots=roots):
            events.append("called")

        ref = weakref.ref(node, callback)
        roots.append(ref)
        # A live weakref whose callback owns the target keeps that target alive.
        if keep_ref:
            roots.clear()
        del node, roots, callback
        if not keep_ref:
            del ref
        gc.collect()
        assert events == (["called"] if keep_ref else [])


only_reachable_weakrefs_run_callbacks()

print("ok")
