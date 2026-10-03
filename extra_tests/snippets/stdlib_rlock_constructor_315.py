"""RLock arguments belong to an overridden initializer, as in CPython 3.15."""

from _thread import RLock


class Plain(RLock):
    pass


class CustomInit(RLock):
    def __init__(self, *args, **kwargs):
        self.received = (args, kwargs)


class InheritedInit(CustomInit):
    pass


class ExplicitBaseInit(RLock):
    __init__ = RLock.__init__


def assert_no_arguments(cls):
    assert not cls().locked()
    for args, kwargs, message in (
        ((1,), {}, "RLock() takes no positional arguments"),
        ((), {"x": 1}, "RLock() takes no keyword arguments"),
        ((1,), {"x": 2}, "RLock() takes no positional arguments"),
    ):
        for construct in (cls, lambda *a, **kw: RLock.__new__(cls, *a, **kw)):
            try:
                construct(*args, **kwargs)
            except TypeError as error:
                assert str(error) == message
            else:
                raise AssertionError(
                    "RLock accepted arguments without a custom initializer"
                )


for cls in (RLock, Plain, ExplicitBaseInit):
    assert_no_arguments(cls)

for cls in (CustomInit, InheritedInit):
    lock = cls(1, x=2)
    assert lock.received == ((1,), {"x": 2})
    assert not lock.locked()
    assert lock.acquire(False)
    assert lock.locked()
    lock.release()
    assert not lock.locked()
    uninitialized = RLock.__new__(cls, 1, x=2)
    assert not hasattr(uninitialized, "received")

# Updating or deleting __init__ must use the current slot, including inheritance.
Plain.__init__ = CustomInit.__init__
assert Plain(1, x=2).received == ((1,), {"x": 2})
del Plain.__init__
assert_no_arguments(Plain)
CustomInit.__init__ = RLock.__init__
assert_no_arguments(CustomInit)
assert_no_arguments(InheritedInit)
