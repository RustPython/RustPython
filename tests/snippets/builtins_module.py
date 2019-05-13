from testutils import assertRaises

assert '__builtins__' in globals()
# assert type(__builtins__).__name__ == 'module'
with assertRaises(AttributeError):
    __builtins__.__builtins__

__builtins__.x = 'new'
assert x == 'new'

exec('assert "__builtins__" in globals()', dict())
exec('assert __builtins__ == 7', {'__builtins__': 7})
exec('assert not isinstance(__builtins__, dict)')
exec('assert isinstance(__builtins__, dict)', {})

namespace = {}
exec('', namespace)
assert namespace['__builtins__'] == __builtins__.__dict__

# with assertRaises(NameError):
#     exec('print(__builtins__)', {'__builtins__': {}})

# __builtins__ is deletable but names are alive
del __builtins__
with assertRaises(NameError):
    __builtins__

assert print
