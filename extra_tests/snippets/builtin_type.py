from testutils import assert_raises


# Spec: https://docs.python.org/2/library/types.html
print(None)
# TypeType
# print(True) # LOAD_NAME???
print(1)
# print(1L) # Long
print(1.1)
# ComplexType
print("abc")
# print(u"abc")
# Structural below
print((1, 2)) # Tuple can be any length, but fixed after declared
x = (1,2)
print(x[0]) # Tuple can be any length, but fixed after declared
print([1, 2, 3])
# print({"first":1,"second":2})

print(int(1))
print(int(1.2))
print(float(1))
print(float(1.2))

assert type(1 - 2) is int
assert type(2 / 3) is float
x = 1
assert type(x) is int
assert type(x - 1) is int

a = bytes([1, 2, 3])
print(a)
b = bytes([1, 2, 3])
assert a == b

with assert_raises(TypeError):
    bytes([object()])

with assert_raises(TypeError):
    bytes(1.0)

with assert_raises(ValueError):
    bytes(-1)

a = bytearray([1, 2, 3])
# assert a[1] == 2

assert int() == 0

a = complex(2, 4)
assert type(a) is complex
assert type(a + a) is complex
assert repr(a) == '(2+4j)'
a = 10j
assert repr(a) == '10j'

a = 1
assert a.conjugate() == a

a = 12345

b = a*a*a*a*a*a*a*a
assert b.bit_length() == 109


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

A.__qualname__ = 'AA'
B.__qualname__ = 'b'
assert A.__qualname__ == 'AA'
assert B.__qualname__ == 'b'
with assert_raises(TypeError):
    del D.__qualname__
with assert_raises(TypeError):
    C.__qualname__ = 123
with assert_raises(TypeError):
    del int.__qualname__

from testutils import assert_raises

import platform
if platform.python_implementation() == 'RustPython':
    gc = None
else:
    import gc

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

if gc:
    # gc sweep is needed here for CPython...
    gc.collect()
    # ...while RustPython doesn't have `gc` yet. 

if gc:
    # D.__new__ is a method bound to the D type, so just deleting D
    # from globals won't actually invalidate the weak reference that
    # subclasses holds. TODO: implement a proper tracing gc
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


assert type(Exception.args).__name__ == 'getset_descriptor'
assert type(None).__bool__(None) is False

class A:
    pass

class B:
    pass

a = A()
a.__class__ = B
assert isinstance(a, B)

b = 1
with assert_raises(TypeError):
    b.__class__ = B


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


# Regression to
# https://github.com/RustPython/RustPython/issues/2776

assert repr(BQ.one).startswith('<function BQ.one at 0x')
assert repr(BQ.one_st).startswith('<function BQ.one_st at 0x')

assert repr(BQ.two).startswith('<function AQ.two at 0x')
assert repr(BQ.two_st).startswith('<function AQ.two_st at 0x')

assert repr(BQ.three).startswith('<function BQ.three at 0x')
assert repr(BQ.three_st).startswith('<function BQ.three_st at 0x')


def my_repr_func():
    pass

assert repr(my_repr_func).startswith('<function my_repr_func at 0x')
