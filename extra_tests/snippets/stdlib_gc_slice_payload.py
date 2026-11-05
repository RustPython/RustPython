import gc
import weakref


class Target:
    pass


target = Target()
reference = weakref.ref(target)
value = slice(None, target, None)
del target
gc.collect()
assert reference() is value.stop

del value
gc.collect()
assert reference() is None, "a cached slice retained its stop argument"

# Reusing cached storage must initialize a fresh payload.
assert slice(1, 7, 2).indices(10) == (1, 7, 2)
