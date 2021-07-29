assert type.__module__ == 'builtins'
assert type.__qualname__ == 'type'
assert type.__name__ == 'type'
assert isinstance(type.__doc__, str)


class A(type):
    pass


class B(type):
    __module__ = 'b'
    __qualname__ = 'BB'


assert A.__module__ == '__main__'
assert A.__qualname__ == 'A'
assert B.__module__ == 'b'
assert B.__qualname__ == 'BB'


# Regression to
# https://github.com/RustPython/RustPython/issues/2310
import builtins
assert builtins.iter.__class__.__module__ == 'builtins'
assert builtins.iter.__class__.__qualname__ == 'builtin_function_or_method'

try:
    builtins.iter.__class__.__docs__
except AttributeError:
    assert True
else:
    assert False, 'Attribute error was not raised'
