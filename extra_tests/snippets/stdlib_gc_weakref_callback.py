"""Weak references own their callbacks, but never own their referents."""

import gc
import weakref


class Target:
    pass


class CallableTarget:
    def __call__(self):
        return 42


def check_callback_cycle(target, make_reference):
    def create():
        holder = []

        def callback(reference):
            holder.append(reference)

        reference = make_reference(target, callback)
        holder.append(reference)
        return weakref.ref(callback)

    reference = create()
    gc.collect()
    assert reference() is None


check_callback_cycle(Target(), weakref.ref)
check_callback_cycle(Target(), weakref.proxy)
check_callback_cycle(CallableTarget(), weakref.proxy)

called = []
target = Target()
target.cycle = target
reference = weakref.ref(target, lambda ref: called.append(ref() is None))
del target
gc.collect()
assert reference() is None
assert called == [True]
print("ok")
