from _weakref import ref, proxy
from testutils import assert_raises


class X:
    pass


a = X()
b = ref(a)

assert callable(b)
assert b() is a


class G:
    def __init__(self, h):
        self.h = h


g = G(5)
p = proxy(g)

assert p.h == 5

del g

assert_raises(ReferenceError, lambda: p.h)
