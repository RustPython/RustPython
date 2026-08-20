import sys

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

# Hashing a deeply nested tuple must not run off the native stack: the hash
# slot dispatch is what recurses, so that is where the depth is checked.

if sys.implementation.name == "rustpython":
    # Deep enough to reach the native stack guard; CPython, which also runs
    # this snippet, dies on the same value.
    deep_tuple = ()
    for _ in range(100_000):
        deep_tuple = (deep_tuple,)
    with assert_raises(RecursionError):
        hash(deep_tuple)
    # a dict key and a set member are hashed on insertion, same dispatch
    with assert_raises(RecursionError):
        {deep_tuple: 1}
    with assert_raises(RecursionError):
        {deep_tuple}
