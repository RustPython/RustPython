import _py_abc
import abc

from testutils import assert_raises


class CustomInterface(abc.ABC):
    @abc.abstractmethod
    def a(self):
        pass

    @classmethod
    def __subclasshook__(cls, subclass):
        return NotImplemented


with assert_raises(TypeError):
    CustomInterface()


class Concrete:
    def a(self):
        pass


CustomInterface.register(Concrete)


class SubConcrete(Concrete):
    pass


assert issubclass(Concrete, CustomInterface)
assert issubclass(SubConcrete, CustomInterface)
assert not issubclass(tuple, CustomInterface)

assert isinstance(Concrete(), CustomInterface)
assert isinstance(SubConcrete(), CustomInterface)
assert not isinstance((), CustomInterface)


def check_missing_class_attribute(meta):
    class Interface(metaclass=meta):
        pass

    class NoClassAttribute:
        @property
        def __class__(self):
            raise AttributeError("no class attribute")

    instance = NoClassAttribute()
    assert not isinstance(instance, Interface)
    Interface.register(NoClassAttribute)
    assert isinstance(instance, Interface)

    class_error = RuntimeError("class lookup failed")

    class BrokenClassAttribute:
        @property
        def __class__(self):
            raise class_error

    with assert_raises(RuntimeError) as caught:
        isinstance(BrokenClassAttribute(), Interface)
    assert caught.exception is class_error


for meta in (abc.ABCMeta, _py_abc.ABCMeta):
    check_missing_class_attribute(meta)
