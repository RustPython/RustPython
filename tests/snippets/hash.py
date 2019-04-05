
from testutils import assertRaises


class A:
    pass


assert type(hash(None)) is int
assert type(hash(object())) is int
assert type(hash(A())) is int
assert type(hash(1)) is int
assert type(hash(1.1)) is int
assert type(hash("")) is int

with assertRaises(TypeError):
    hash({})

with assertRaises(TypeError):
    hash(set())

with assertRaises(TypeError):
    hash([])
