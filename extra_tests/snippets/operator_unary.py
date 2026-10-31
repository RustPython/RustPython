from testutils import assert_raises


class A:
    def __neg__(self):
        return "neg"

    def __pos__(self):
        return "pos"

    def __abs__(self):
        return "abs"

    def __invert__(self):
        return "invert"


a = A()
assert -a == "neg"
assert +a == "pos"
assert abs(a) == "abs"
assert ~a == "invert"


class I(int):
    def __neg__(self):
        return "I.neg"


assert -I(3) == "I.neg"
assert +I(3) == 3
assert abs(I(-3)) == 3
assert ~I(3) == -4


# Slots follow methods assigned or deleted after class creation.
class Late:
    pass


with assert_raises(TypeError):
    -Late()
Late.__neg__ = lambda self: "late"
assert -Late() == "late"
del Late.__neg__
with assert_raises(TypeError):
    -Late()

del I.__neg__
assert -I(3) == -3

# Only the type is consulted, not the instance.
i = Late()
i.__neg__ = lambda: "instance"
with assert_raises(TypeError):
    -i


class Meta(type):
    def __neg__(cls):
        return cls.__name__


class WithMeta(metaclass=Meta):
    pass


assert -WithMeta == "WithMeta"

with assert_raises(TypeError) as cm:
    -"a"
assert str(cm.exception) == "bad operand type for unary -: 'str'"
with assert_raises(TypeError) as cm:
    abs([])
assert str(cm.exception) == "bad operand type for abs(): 'list'"
