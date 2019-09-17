from testutils import assert_raises

assert '__builtins__' in globals()
# assert type(__builtins__).__name__ == 'module'
with assert_raises(AttributeError):
    __builtins__.__builtins__

__builtins__.x = 'new'
assert x == 'new'  # noqa: F821

exec('assert "__builtins__" in globals()', dict())
exec('assert __builtins__ == 7', {'__builtins__': 7})
exec('assert not isinstance(__builtins__, dict)')
exec('assert isinstance(__builtins__, dict)', {})

namespace = {}
exec('', namespace)
assert namespace['__builtins__'] == __builtins__.__dict__

# with assert_raises(NameError):
#     exec('print(__builtins__)', {'__builtins__': {}})

# __builtins__ is deletable but names are alive
del __builtins__
with assert_raises(NameError):
    __builtins__  # noqa: F821

assert print
