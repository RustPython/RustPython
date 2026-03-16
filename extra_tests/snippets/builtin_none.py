assert None is None

y = None
x = None
assert x is y


def none():
    pass


def none2():
    return None


assert none() is none()
assert none() is x

assert none() is none2()

assert str(None) == "None"
assert repr(None) == "None"
assert type(None)() is None

assert None.__eq__(3) is NotImplemented
assert None.__ne__(3) is NotImplemented
assert None.__eq__(None) is True
assert None.__ne__(None) is False
assert None.__lt__(3) is NotImplemented
assert None.__le__(3) is NotImplemented
assert None.__gt__(3) is NotImplemented
assert None.__ge__(3) is NotImplemented

none_type_dict = type(None).__dict__
for name in ("__eq__", "__ne__", "__lt__", "__le__", "__gt__", "__ge__", "__hash__"):
    assert name in none_type_dict
    assert none_type_dict[name] is not object.__dict__[name]
    assert type(none_type_dict[name]).__name__ == "wrapper_descriptor"

assert hash(None) & 0xFFFFFFFF == 0xFCA86420
