
class A:
    pass


class B(A):
    pass


assert issubclass(A, A)
assert issubclass(B, A)
assert not issubclass(A, B)


class MCNotSubClass(type):
    def __subclasscheck__(self, subclass):
        return False


class NotSubClass(metaclass=MCNotSubClass):
    pass


class InheritedNotSubClass(NotSubClass):
    pass


assert not issubclass(A, NotSubClass)
assert not issubclass(NotSubClass, NotSubClass)
assert not issubclass(InheritedNotSubClass, NotSubClass)
assert not issubclass(NotSubClass, InheritedNotSubClass)


class MCAlwaysSubClass(type):
    def __subclasscheck__(self, subclass):
        return True


class AlwaysSubClass(metaclass=MCAlwaysSubClass):
    pass


class InheritedAlwaysSubClass(AlwaysSubClass):
    pass


assert issubclass(A, AlwaysSubClass)
assert issubclass(AlwaysSubClass, AlwaysSubClass)
assert issubclass(InheritedAlwaysSubClass, AlwaysSubClass)
assert issubclass(AlwaysSubClass, InheritedAlwaysSubClass)


class MCAVirtualSubClass(type):
    def __subclasscheck__(self, subclass):
        return subclass is A


class AVirtualSubClass(metaclass=MCAVirtualSubClass):
    pass


assert issubclass(A, AVirtualSubClass)
assert not isinstance(B, AVirtualSubClass)
