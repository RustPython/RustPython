# simple values
assert max(0, 0) == 0
assert max(1, 0) == 1
assert max(1., 0.) == 1.
assert max(-1, 0) == 0
assert max(1, 2, 3) == 3

# iterables
assert max([1, 2, 3]) == 3
assert max((1, 2, 3)) == 3
assert max({
    "a": 0,
    "b": 1,
}) == "b"
assert max([1, 2], default=0) == 2
assert max([], default=0) == 0
try:
    max([])
except ValueError:
    pass
else:
    assert False, "ValueError was not raised"

# key parameter
assert max(1, 2, -3, key=abs) == -3
assert max([1, 2, -3], key=abs) == -3

# no argument
try:
    max()
except TypeError:
    pass
else:
    assert False, "TypeError was not raised"

# one non-iterable argument
try:
    max(1)
except TypeError:
    pass
else:
    assert False, "TypeError was not raised"


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
assert max(first, second) == second
assert max([first, second]) == second


class MyNotComparable():
    pass


try:
    max(MyNotComparable(), MyNotComparable())
except TypeError:
    pass
else:
    assert False, "TypeError was not raised"
