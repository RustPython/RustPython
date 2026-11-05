"""A slot descriptor owns its class, including when the class becomes garbage."""

import gc
import weakref


def create():
    class Temporary:
        __slots__ = ("value",)

    return weakref.ref(Temporary), Temporary.value


reference, descriptor = create()
gc.collect()
assert reference() is descriptor.__objclass__
instance = descriptor.__objclass__()
descriptor.__set__(instance, 42)
assert descriptor.__get__(instance) == 42

del instance, descriptor
gc.collect()
assert reference() is None
print("ok")
