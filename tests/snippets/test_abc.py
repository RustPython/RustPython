import abc

from testutils import assertRaises


class CustomInterface(abc.ABC):
    @abc.abstractmethod
    def a(self):
        pass

    @classmethod
    def __subclasshook__(cls, subclass):
        return NotImplemented


# TODO raise an error if there are in any abstract methods not fulfilled
# with assertRaises(TypeError):
#     CustomInterface()


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
