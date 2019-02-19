import sys
from _weakref import ref


data_holder = {}


class X:
    def __init__(self, param=0):
        self.param = param

    def __str__(self):
        return f"param: {self.param}"


a = X()
b = ref(a)


def callback(weak_ref):
    assert weak_ref is c
    assert b() is None, 'reference to same object is dead'
    assert c() is None, 'reference is dead'
    data_holder['first'] = True


c = ref(a, callback)


def never_callback(_weak_ref):
    data_holder['never'] = True


# weakref should be cleaned up before object, so callback is never called
ref(a, never_callback)

assert callable(b)
assert b() is a

assert 'first' not in data_holder
del a
assert b() is None
assert 'first' in data_holder
assert 'never' not in data_holder

# TODO proper detection of RustPython if sys.implementation.name == 'RustPython':
if not hasattr(sys, 'implementation'):
    # implementation detail that the object isn't dropped straight away
    # this tests that when an object is resurrected it still acts as normal
    delayed_drop = X(5)
    delayed_drop_ref = ref(delayed_drop)

    delayed_drop = None

    assert delayed_drop_ref() is not None
    value = delayed_drop_ref()
    del delayed_drop  # triggers process_deletes

    assert str(value) == "param: 5"

