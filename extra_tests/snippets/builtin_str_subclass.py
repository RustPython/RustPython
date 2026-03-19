from testutils import assert_raises

x = "An interesting piece of text"
assert x is str(x)


class Stringy(str):
    def __new__(cls, value=""):
        return str.__new__(cls, value)

    def __init__(self, value):
        self.x = "substr"


y = Stringy(1)
assert type(y) is Stringy, "Type of Stringy should be stringy"
assert type(str(y)) is str, "Str of a str-subtype should be a str."

assert y + " other" == "1 other"
assert y.x == "substr"


class ReprStrSubclass(str):
    pass


class WithStr:
    def __init__(self, value):
        self.value = value

    def __str__(self):
        return self.value


class WithRepr:
    def __init__(self, value):
        self.value = value

    def __repr__(self):
        return self.value


str_value = ReprStrSubclass("abc")
assert str(WithStr(str_value)) is str_value

repr_value = ReprStrSubclass("<abc>")
assert str(WithRepr(repr_value)) is repr_value

## Base strings currently get an attribute dict, but shouldn't.
# with assert_raises(AttributeError):
#     "hello".x = 5
