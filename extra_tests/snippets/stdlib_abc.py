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


class NoClassAttribute:
    @property
    def __class__(self):
        raise AttributeError("no class attribute")


instance = NoClassAttribute()
assert not isinstance(instance, CustomInterface)
CustomInterface.register(NoClassAttribute)
assert isinstance(instance, CustomInterface)

class_error = RuntimeError("class lookup failed")


class BrokenClassAttribute:
    @property
    def __class__(self):
        raise class_error


with assert_raises(RuntimeError) as caught:
    isinstance(BrokenClassAttribute(), CustomInterface)
assert caught.exception is class_error
