from testutils import assert_raises


class A:
    pass


assert type(hash(None)) is int
assert type(hash(object())) is int
assert type(hash(A())) is int
assert type(hash(1)) is int
assert type(hash(1.1)) is int
assert type(hash("")) is int


class Evil:
    def __hash__(self):
        return 1 << 63


assert hash(Evil()) == 4

with assert_raises(TypeError):
    hash({})

with assert_raises(TypeError):
    hash(set())

with assert_raises(TypeError):
    hash([])
