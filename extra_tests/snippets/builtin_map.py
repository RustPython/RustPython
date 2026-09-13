a = list(map(str, [1, 2, 3]))
assert a == ["1", "2", "3"]


b = list(map(lambda x, y: x + y, [1, 2, 4], [3, 5]))
assert b == [4, 7]

assert type(map(lambda x: x, [])) == map


# test infinite iterator
class Counter(object):
    counter = 0

    def __next__(self):
        self.counter += 1
        return self.counter

    def __iter__(self):
        return self


it = map(lambda x: x + 1, Counter())
assert next(it) == 2
assert next(it) == 3


def mapping(x):
    if x == 0:
        raise StopIteration()
    return x


assert list(map(mapping, [1, 2, 0, 4, 5])) == [1, 2]


# map does not report a length hint, so a chain of them is not walked to
# answer for one.
import operator

assert not hasattr(map(lambda x: x, [1, 2, 3]), "__length_hint__")
assert operator.length_hint(map(lambda x: x, [1, 2, 3])) == 0

it = iter([1, 2, 3])
for _ in range(10000):
    it = map(lambda x: x, it)
assert operator.length_hint(it) == 0
