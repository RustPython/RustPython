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

assert MyTypeWithMethod.method.__name__ == 'method'
assert MyTypeWithMethod().method.__name__ == 'method'
assert MyTypeWithMethod.clsmethod.__name__ == 'clsmethod'
assert MyTypeWithMethod().clsmethod.__name__ == 'clsmethod'
assert MyTypeWithMethod.stmethod.__name__ == 'stmethod'
assert MyTypeWithMethod().stmethod.__name__ == 'stmethod'

assert MyTypeWithMethod.method.__qualname__ == 'MyTypeWithMethod.method'
assert MyTypeWithMethod().method.__qualname__ == 'MyTypeWithMethod.method'
assert MyTypeWithMethod.clsmethod.__qualname__ == 'MyTypeWithMethod.clsmethod'
assert MyTypeWithMethod().clsmethod.__qualname__ == 'MyTypeWithMethod.clsmethod'
assert MyTypeWithMethod.stmethod.__qualname__ == 'MyTypeWithMethod.stmethod'
assert MyTypeWithMethod().stmethod.__qualname__ == 'MyTypeWithMethod.stmethod'

assert MyTypeWithMethod.N.m.__name__ == 'm'
assert MyTypeWithMethod().N.m.__name__ == 'm'
assert MyTypeWithMethod.N.c.__name__ == 'c'
assert MyTypeWithMethod().N.c.__name__ == 'c'
assert MyTypeWithMethod.N.s.__name__ == 's'
assert MyTypeWithMethod().N.s.__name__ == 's'

assert MyTypeWithMethod.N.m.__qualname__ == 'MyTypeWithMethod.N.m'
assert MyTypeWithMethod().N.m.__qualname__ == 'MyTypeWithMethod.N.m'
assert MyTypeWithMethod.N.c.__qualname__ == 'MyTypeWithMethod.N.c'
assert MyTypeWithMethod().N.c.__qualname__ == 'MyTypeWithMethod.N.c'
assert MyTypeWithMethod.N.s.__qualname__ == 'MyTypeWithMethod.N.s'
assert MyTypeWithMethod().N.s.__qualname__ == 'MyTypeWithMethod.N.s'

assert MyTypeWithMethod.N().m.__name__ == 'm'
assert MyTypeWithMethod().N().m.__name__ == 'm'
assert MyTypeWithMethod.N().c.__name__ == 'c'
assert MyTypeWithMethod().N().c.__name__ == 'c'
assert MyTypeWithMethod.N().s.__name__ == 's'
assert MyTypeWithMethod().N.s.__name__ == 's'

assert MyTypeWithMethod.N().m.__qualname__ == 'MyTypeWithMethod.N.m'
assert MyTypeWithMethod().N().m.__qualname__ == 'MyTypeWithMethod.N.m'
assert MyTypeWithMethod.N().c.__qualname__ == 'MyTypeWithMethod.N.c'
assert MyTypeWithMethod().N().c.__qualname__ == 'MyTypeWithMethod.N.c'
assert MyTypeWithMethod.N().s.__qualname__ == 'MyTypeWithMethod.N.s'
assert MyTypeWithMethod().N().s.__qualname__ == 'MyTypeWithMethod.N.s'


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


# Regression to
# https://github.com/RustPython/RustPython/issues/2786

assert object.__new__.__name__ == '__new__'
assert object.__new__.__qualname__ == 'object.__new__'
assert object.__subclasshook__.__name__ == '__subclasshook__'
assert object.__subclasshook__.__qualname__ == 'object.__subclasshook__'
assert type.__new__.__name__ == '__new__'
assert type.__new__.__qualname__ == 'type.__new__'


class AQ:
    # To be overridden:

    def one(self):
        pass

    @classmethod
    def one_cls(cls):
        pass

    @staticmethod
    def one_st():
        pass

    # To be inherited:

    def two(self):
        pass

    @classmethod
    def two_cls(cls):
        pass

    @staticmethod
    def two_st():
        pass


class BQ(AQ):
    def one(self):
        pass

    @classmethod
    def one_cls(cls):
        pass

    @staticmethod
    def one_st():
        pass

    # Extras, defined in subclass:

    def three(self):
        pass

    @classmethod
    def three_cls(cls):
        pass

    @staticmethod
    def three_st():
        pass

assert AQ.one.__name__ == 'one'
assert AQ().one.__name__ == 'one'
assert AQ.one_cls.__name__ == 'one_cls'
assert AQ().one_cls.__name__ == 'one_cls'
assert AQ.one_st.__name__ == 'one_st'
assert AQ().one_st.__name__ == 'one_st'

assert AQ.one.__qualname__ == 'AQ.one'
assert AQ().one.__qualname__ == 'AQ.one'
assert AQ.one_cls.__qualname__ == 'AQ.one_cls'
assert AQ().one_cls.__qualname__ == 'AQ.one_cls'
assert AQ.one_st.__qualname__ == 'AQ.one_st'
assert AQ().one_st.__qualname__ == 'AQ.one_st'

assert AQ.two.__name__ == 'two'
assert AQ().two.__name__ == 'two'
assert AQ.two_cls.__name__ == 'two_cls'
assert AQ().two_cls.__name__ == 'two_cls'
assert AQ.two_st.__name__ == 'two_st'
assert AQ().two_st.__name__ == 'two_st'

assert AQ.two.__qualname__ == 'AQ.two'
assert AQ().two.__qualname__ == 'AQ.two'
assert AQ.two_cls.__qualname__ == 'AQ.two_cls'
assert AQ().two_cls.__qualname__ == 'AQ.two_cls'
assert AQ.two_st.__qualname__ == 'AQ.two_st'
assert AQ().two_st.__qualname__ == 'AQ.two_st'

assert BQ.one.__name__ == 'one'
assert BQ().one.__name__ == 'one'
assert BQ.one_cls.__name__ == 'one_cls'
assert BQ().one_cls.__name__ == 'one_cls'
assert BQ.one_st.__name__ == 'one_st'
assert BQ().one_st.__name__ == 'one_st'

assert BQ.one.__qualname__ == 'BQ.one'
assert BQ().one.__qualname__ == 'BQ.one'
assert BQ.one_cls.__qualname__ == 'BQ.one_cls'
assert BQ().one_cls.__qualname__ == 'BQ.one_cls'
assert BQ.one_st.__qualname__ == 'BQ.one_st'
assert BQ().one_st.__qualname__ == 'BQ.one_st'

assert BQ.two.__name__ == 'two'
assert BQ().two.__name__ == 'two'
assert BQ.two_cls.__name__ == 'two_cls'
assert BQ().two_cls.__name__ == 'two_cls'
assert BQ.two_st.__name__ == 'two_st'
assert BQ().two_st.__name__ == 'two_st'

assert BQ.two.__qualname__ == 'AQ.two'
assert BQ().two.__qualname__ == 'AQ.two'
assert BQ.two_cls.__qualname__ == 'AQ.two_cls'
assert BQ().two_cls.__qualname__ == 'AQ.two_cls'
assert BQ.two_st.__qualname__ == 'AQ.two_st'
assert BQ().two_st.__qualname__ == 'AQ.two_st'

assert BQ.three.__name__ == 'three'
assert BQ().three.__name__ == 'three'
assert BQ.three_cls.__name__ == 'three_cls'
assert BQ().three_cls.__name__ == 'three_cls'
assert BQ.three_st.__name__ == 'three_st'
assert BQ().three_st.__name__ == 'three_st'

assert BQ.three.__qualname__ == 'BQ.three'
assert BQ().three.__qualname__ == 'BQ.three'
assert BQ.three_cls.__qualname__ == 'BQ.three_cls'
assert BQ().three_cls.__qualname__ == 'BQ.three_cls'
assert BQ.three_st.__qualname__ == 'BQ.three_st'
assert BQ().three_st.__qualname__ == 'BQ.three_st'


class ClassWithNew:
    def __new__(cls, *args, **kwargs):
        return super().__new__(cls, *args, **kwargs)

    class N:
        def __new__(cls, *args, **kwargs):
            return super().__new__(cls, *args, **kwargs)


assert ClassWithNew.__new__.__qualname__ == 'ClassWithNew.__new__'
assert ClassWithNew().__new__.__qualname__ == 'ClassWithNew.__new__'
assert ClassWithNew.__new__.__name__ == '__new__'
assert ClassWithNew().__new__.__name__ == '__new__'

assert ClassWithNew.N.__new__.__qualname__ == 'ClassWithNew.N.__new__'
assert ClassWithNew().N.__new__.__qualname__ == 'ClassWithNew.N.__new__'
assert ClassWithNew.N.__new__.__name__ == '__new__'
assert ClassWithNew().N.__new__.__name__ == '__new__'
assert ClassWithNew.N().__new__.__qualname__ == 'ClassWithNew.N.__new__'
assert ClassWithNew().N().__new__.__qualname__ == 'ClassWithNew.N.__new__'
assert ClassWithNew.N().__new__.__name__ == '__new__'
assert ClassWithNew().N().__new__.__name__ == '__new__'


# Regression to:
# https://github.com/RustPython/RustPython/issues/2762

assert type.__prepare__() == {}
assert type.__prepare__('name') == {}
assert type.__prepare__('name', object) == {}
assert type.__prepare__('name', (bytes, str)) == {}
assert type.__prepare__(a=1, b=2) == {}
assert type.__prepare__('name', (object, int), kw=True) == {}

# Previously we needed `name` to be `str`:
assert type.__prepare__(1) == {}

assert int.__prepare__() == {}
assert int.__prepare__('name', (object, int), kw=True) == {}


# Regression to
# https://github.com/RustPython/RustPython/issues/2790

# `#[pyproperty]`
assert BaseException.args.__qualname__ == 'BaseException.args'
# class extension without `#[pyproperty]` override
assert Exception.args.__qualname__ == 'BaseException.args'
# dynamic with `.new_readonly_getset`
assert SyntaxError.msg.__qualname__ == 'SyntaxError.msg'


# Regression to
# https://github.com/RustPython/RustPython/issues/2794

assert type.__subclasshook__.__qualname__ == 'type.__subclasshook__'
assert object.__subclasshook__.__qualname__ == 'object.__subclasshook__'
