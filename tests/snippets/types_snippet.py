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

try: # gc sweep is needed here for CPython...
    import gc; gc.collect()
except: # ...while RustPython doesn't have `gc` yet.
    pass

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
