from _weakref import proxy, ref

from testutils import assert_raises


class X:
    pass


a = X()
b = ref(a)

assert callable(b)
assert b() is a

# Test __callback__ property
assert b.__callback__ is None, "weakref without callback should have __callback__ == None"

callback = lambda r: None
c = ref(a, callback)
assert c.__callback__ is callback, "weakref with callback should return the callback"

# Test __callback__ is read-only
try:
    c.__callback__ = lambda r: None
    assert False, "Setting __callback__ should raise AttributeError"
except AttributeError:
    pass

# Test __callback__ after referent deletion
x = X()
cb = lambda r: None
w = ref(x, cb)
assert w.__callback__ is cb
del x
assert w.__callback__ is None, "__callback__ should be None after referent is collected"


class G:
    def __init__(self, h):
        self.h = h


g = G(5)
p = proxy(g)

assert p.h == 5

del g

assert_raises(ReferenceError, lambda: p.h)
