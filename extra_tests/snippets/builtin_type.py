assert type.__module__ == 'builtins'
assert type.__qualname__ == 'type'
assert type.__name__ == 'type'
assert isinstance(type.__doc__, str)

# Regression to
# https://github.com/RustPython/RustPython/issues/2310
import builtins
assert builtins.iter.__class__.__module__ == 'builtins'
