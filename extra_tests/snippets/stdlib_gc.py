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

print("ok")
