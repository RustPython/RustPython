from testutils import assertRaises

assert '__builtins__' in globals()
# assert type(__builtins__).__name__ == 'module'
with assertRaises(AttributeError):
    __builtins__.__builtins__

__builtins__.x = 'new'
assert x == 'new'

# __builtins__ is deletable but names are alive
del __builtins__
with assertRaises(NameError):
    __builtins__

assert print
