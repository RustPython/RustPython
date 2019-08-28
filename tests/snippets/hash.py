
from testutils import assert_raises


class A:
    pass


assert type(hash(None)) is int
assert type(hash(object())) is int
assert type(hash(A())) is int
assert type(hash(1)) is int
assert type(hash(1.1)) is int
assert type(hash("")) is int

with assert_raises(TypeError):
    hash({})

with assert_raises(TypeError):
    hash(set())

with assert_raises(TypeError):
    hash([])
