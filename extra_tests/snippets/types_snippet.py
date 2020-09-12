from testutils import assert_raises

import platform
if platform.python_implementation() == 'RustPython':
    gc = None
else:
    import gc

assert type(type) is type
assert type(object) is type
assert type(object()) is object

new_type = type('New', (object,), {})

assert type(new_type) is type
assert type(new_type()) is new_type

metaclass = type('MCl', (type,), {})
cls = metaclass('Cls', (object,), {})
inst = cls()

assert type(inst) is cls
assert type(cls) is metaclass
assert type(metaclass) is type

assert issubclass(metaclass, type)
assert isinstance(cls, type)

assert inst.__class__ is cls
assert cls.__class__ is metaclass
assert metaclass.__class__ is type
assert type.__class__ is type
assert None.__class__ is type(None)

assert isinstance(type, type)
assert issubclass(type, type)

assert not isinstance(type, (int, float))
assert isinstance(type, (int, object))

assert not issubclass(type, (int, float))
assert issubclass(type, (int, type))

class A: pass
class B(A): pass
class C(A): pass
class D(B, C): pass

assert A.__subclasses__() == [B, C]
assert B.__subclasses__() == [D]
assert C.__subclasses__() == [D]
assert D.__subclasses__() == []

assert D.__bases__ == (B, C)
assert A.__bases__ == (object,)
assert B.__bases__ == (A,)


del D

if gc:
    # gc sweep is needed here for CPython...
    gc.collect()
    # ...while RustPython doesn't have `gc` yet. 

if gc:
    # D.__new__ is a method bound to the D type, so just deleting D
    # from globals won't actually invalidate the weak reference that
    # subclasses holds. TODO: implement a proper tracing gc
    assert B.__subclasses__() == []
    assert C.__subclasses__() == []

assert type in object.__subclasses__()

assert cls.__name__ == 'Cls'

# mro
assert int.mro() == [int, object]
assert bool.mro() == [bool, int, object]
assert object.mro() == [object]

class A:
    pass

class B(A):
    pass

assert A.mro() == [A, object]
assert B.mro() == [B, A, object]

class AA:
    pass

class BB(AA):
    pass

class C(B, BB):
    pass

assert C.mro() == [C, B, A, BB, AA, object]


assert type(Exception.args).__name__ == 'getset_descriptor'
assert type(None).__bool__(None) is False

class A:
    pass

class B:
    pass

a = A()
a.__class__ = B
assert isinstance(a, B)

b = 1
with assert_raises(TypeError):
    b.__class__ = B
