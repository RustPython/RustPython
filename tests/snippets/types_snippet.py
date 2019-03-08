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
