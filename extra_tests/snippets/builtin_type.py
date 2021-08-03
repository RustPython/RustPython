assert type.__module__ == 'builtins'
assert type.__qualname__ == 'type'
assert type.__name__ == 'type'
assert isinstance(type.__doc__, str)
assert object.__qualname__ == 'object'
assert int.__qualname__ == 'int'


class A(type):
    pass


class B(type):
    __module__ = 'b'
    __qualname__ = 'BB'


class C:
    pass


class D:
    __module__ = 'd'
    __qualname__ = 'DD'


assert A.__module__ == '__main__'
assert A.__qualname__ == 'A'
assert B.__module__ == 'b'
assert B.__qualname__ == 'BB'
assert C.__module__ == '__main__'
assert C.__qualname__ == 'C'
assert D.__module__ == 'd'
assert D.__qualname__ == 'DD'


# Regression to
# https://github.com/RustPython/RustPython/issues/2310
import builtins
assert builtins.iter.__class__.__module__ == 'builtins'
assert builtins.iter.__class__.__qualname__ == 'builtin_function_or_method'

assert iter.__class__.__module__ == 'builtins'
assert iter.__class__.__qualname__ == 'builtin_function_or_method'
assert type(iter).__module__ == 'builtins'
assert type(iter).__qualname__ == 'builtin_function_or_method'


# Regression to
# https://github.com/RustPython/RustPython/issues/2767

# Marked as `#[pymethod]`:
assert str.replace.__qualname__ == 'str.replace'
assert str().replace.__qualname__ == 'str.replace'
assert int.to_bytes.__qualname__ == 'int.to_bytes'
assert int().to_bytes.__qualname__ == 'int.to_bytes'

# Marked as `#[pyclassmethod]`:
assert dict.fromkeys.__qualname__ == 'dict.fromkeys'
assert object.__init_subclass__.__qualname__ == 'object.__init_subclass__'

# Dynamic with `#[extend_class]`:
assert bytearray.maketrans.__qualname__ == 'bytearray.maketrans'


# Third-party:
class MyTypeWithMethod:
    def method(self):
        pass

    @classmethod
    def clsmethod(cls):
        pass

    @staticmethod
    def stmethod():
        pass

    class N:
        def m(self):
            pass

        @classmethod
        def c(cls):
            pass

        @staticmethod
        def s():
            pass

assert MyTypeWithMethod.method.__qualname__ == 'MyTypeWithMethod.method'
assert MyTypeWithMethod().method.__qualname__ == 'MyTypeWithMethod.method'
assert MyTypeWithMethod.clsmethod.__qualname__ == 'MyTypeWithMethod.clsmethod'
assert MyTypeWithMethod().clsmethod.__qualname__ == 'MyTypeWithMethod.clsmethod'
assert MyTypeWithMethod.stmethod.__qualname__ == 'MyTypeWithMethod.stmethod'
assert MyTypeWithMethod().stmethod.__qualname__ == 'MyTypeWithMethod.stmethod'

assert MyTypeWithMethod.N.m.__qualname__ == 'MyTypeWithMethod.N.m'
assert MyTypeWithMethod().N.m.__qualname__ == 'MyTypeWithMethod.N.m'
assert MyTypeWithMethod.N.c.__qualname__ == 'MyTypeWithMethod.N.c'
assert MyTypeWithMethod().N.c.__qualname__ == 'MyTypeWithMethod.N.c'
assert MyTypeWithMethod.N.s.__qualname__ == 'MyTypeWithMethod.N.s'
assert MyTypeWithMethod().N.s.__qualname__ == 'MyTypeWithMethod.N.s'


# Regresesion to
# https://github.com/RustPython/RustPython/issues/2775

assert repr(str.replace) == "<method 'replace' of 'str' objects>"
assert repr(str.replace) == str(str.replace)
assert repr(int.to_bytes) == "<method 'to_bytes' of 'int' objects>"


# Regression to
# https://github.com/RustPython/RustPython/issues/2788

assert iter.__qualname__ == iter.__name__ == 'iter'
assert max.__qualname__ == max.__name__ == 'max'
assert min.__qualname__ ==  min.__name__ == 'min'


def custom_func():
    pass

assert custom_func.__qualname__ == 'custom_func'
