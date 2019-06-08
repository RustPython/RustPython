assert not callable(1)
def f(): pass
assert callable(f)
assert callable(len)
assert callable(lambda: 1)
assert callable(int)

class C:
    def __init__(self):
        # must be defined on class
        self.__call__ = lambda self: 1
    def f(self): pass
assert callable(C)
assert not callable(C())
assert callable(C().f)

class C:
    def __call__(self): pass
assert callable(C())
class C1(C): pass
assert callable(C1())
class C:
    __call__ = 1
# CPython returns true here, but fails when actually calling it
assert callable(C())
