from testutils import assert_raises

# test lists
assert 3 in [1, 2, 3]
assert 3 not in [1, 2]

assert not (3 in [1, 2])
assert not (3 not in [1, 2, 3])

# test strings
assert "foo" in "foobar"
assert "whatever" not in "foobar"

# test bytes
assert b"foo" in b"foobar"
assert b"whatever" not in b"foobar"
assert b"1" < b"2"
assert b"1" <= b"2"
assert b"5" <= b"5"
assert b"4" > b"2"
assert not b"1" >= b"2"
assert b"10" >= b"10"
assert_raises(TypeError, lambda: bytes() > 2)

# test tuple
assert 1 in (1, 2)
assert 3 not in (1, 2)

# test set
assert 1 in set([1, 2])
assert 3 not in set([1, 2])

# test dicts
assert "a" in {"a": 0, "b": 0}
assert "c" not in {"a": 0, "b": 0}
assert 1 in {1: 5, 7: 12}
assert 5 not in {9: 10, 50: 100}
assert True in {True: 5}
assert False not in {True: 5}

# test iter
assert 3 in iter([1, 2, 3])
assert 3 not in iter([1, 2])

# test sequence
assert 1 in range(0, 2)
assert 3 not in range(0, 2)

# test __contains__ in user objects
class MyNotContainingClass():
    pass


assert_raises(TypeError, lambda: 1 in MyNotContainingClass())


class MyContainingClass():
    def __init__(self, value):
        self.value = value

    def __contains__(self, something):
        return something == self.value


assert 2 in MyContainingClass(2)
assert 1 not in MyContainingClass(2)
