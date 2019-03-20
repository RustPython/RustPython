from testutils import assertRaises


class A:
    pass


a = A()
a.b = 10
assert hasattr(a, 'b')
assert a.b == 10

# test override attribute
setattr(a, 'b', 12)
assert a.b == 12
assert getattr(a, 'b') == 12

# test non-existent attribute
with assertRaises(AttributeError):
    _ = a.c

with assertRaises(AttributeError):
    getattr(a, 'c')

assert getattr(a, 'c', 21) == 21

# test set attribute
setattr(a, 'c', 20)
assert hasattr(a, 'c')
assert a.c == 20

# test delete attribute
delattr(a, 'c')
assert not hasattr(a, 'c')
with assertRaises(AttributeError):
    _ = a.c


# test setting attribute on builtin
with assertRaises(AttributeError):
    object().a = 1

with assertRaises(AttributeError):
    setattr(object(), 'a', 2)

attrs = {}


class CustomLookup:
    def __getattr__(self, item):
        return "value_{}".format(item)

    def __setattr__(self, key, value):
        attrs[key] = value


custom = CustomLookup()

assert custom.attr == "value_attr"

custom.a = 2
custom.b = 5
assert attrs['a'] == 2
assert attrs['b'] == 5


class GetRaise:
    def __init__(self, ex):
        self.ex = ex

    def __getattr__(self, item):
        raise self.ex


assert not hasattr(GetRaise(AttributeError()), 'a')
with assertRaises(AttributeError):
    getattr(GetRaise(AttributeError()), 'a')
assert getattr(GetRaise(AttributeError()), 'a', 11) == 11

with assertRaises(KeyError):
    hasattr(GetRaise(KeyError()), 'a')
with assertRaises(KeyError):
    getattr(GetRaise(KeyError()), 'a')
with assertRaises(KeyError):
    getattr(GetRaise(KeyError()), 'a', 11)
