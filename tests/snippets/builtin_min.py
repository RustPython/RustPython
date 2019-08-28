from testutils import assert_raises

# simple values
assert min(0, 0) == 0
assert min(1, 0) == 0
assert min(1., 0.) == 0.
assert min(-1, 0) == -1
assert min(1, 2, 3) == 1

# iterables
assert min([1, 2, 3]) == 1
assert min((1, 2, 3)) == 1
assert min({
    "a": 0,
    "b": 1,
}) == "a"
assert min([1, 2], default=0) == 1
assert min([], default=0) == 0

assert_raises(ValueError, min, [])

# key parameter
assert min(1, 2, -3, key=abs) == 1
assert min([1, 2, -3], key=abs) == 1

# no argument
assert_raises(TypeError, min)

# one non-iterable argument
assert_raises(TypeError, min, 1)


# custom class
class MyComparable():
    nb = 0

    def __init__(self):
        self.my_nb = MyComparable.nb
        MyComparable.nb += 1

    def __gt__(self, other):
        return self.my_nb > other.my_nb


first = MyComparable()
second = MyComparable()
assert min(first, second) == first
assert min([first, second]) == first


class MyNotComparable():
    pass


assert_raises(TypeError, min, MyNotComparable(), MyNotComparable())
