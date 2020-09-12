
class Regular:
    pass


assert isinstance(Regular(), Regular)


class MCNotInstanceOf(type):
    def __instancecheck__(self, instance):
        return False


class NotInstanceOf(metaclass=MCNotInstanceOf):
    pass


class InheritedNotInstanceOf(NotInstanceOf):
    pass


assert not isinstance(Regular(), NotInstanceOf)
assert not isinstance(1, NotInstanceOf)

# weird cpython behaviour if exact match then isinstance return true
assert isinstance(NotInstanceOf(), NotInstanceOf)
assert not NotInstanceOf.__instancecheck__(NotInstanceOf())
assert not isinstance(InheritedNotInstanceOf(), NotInstanceOf)


class MCAlwaysInstanceOf(type):
    def __instancecheck__(self, instance):
        return True


class AlwaysInstanceOf(metaclass=MCAlwaysInstanceOf):
    pass


assert isinstance(AlwaysInstanceOf(), AlwaysInstanceOf)
assert isinstance(Regular(), AlwaysInstanceOf)
assert isinstance(1, AlwaysInstanceOf)


class MCReturnInt(type):
    def __instancecheck__(self, instance):
        return 3


class ReturnInt(metaclass=MCReturnInt):
    pass


assert isinstance("a", ReturnInt) is True

assert isinstance(1, ((int, float,), str))
