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
