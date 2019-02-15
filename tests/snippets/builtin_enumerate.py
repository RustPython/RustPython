assert list(enumerate(['a', 'b', 'c'])) == [(0, 'a'), (1, 'b'), (2, 'c')]

assert type(enumerate([])) == enumerate

assert list(enumerate(['a', 'b', 'c'], -100)) == [(-100, 'a'), (-99, 'b'), (-98, 'c')]
assert list(enumerate(['a', 'b', 'c'], 2**200)) == [(2**200, 'a'), (2**200 + 1, 'b'), (2**200 + 2, 'c')]

a = list([1, 2, 3])
assert len(a) == 3
a.remove(1)
assert len(a) == 2
assert not 1 in a

try:
    a.remove(10)
except ValueError:
    pass
else:
    assert False, "Remove not exist element should raise ValueError"


# test infinite iterator
class Counter(object):
    counter = 0

    def __next__(self):
        self.counter += 1
        return self.counter

    def __iter__(self):
        return self


it = enumerate(Counter())
assert next(it) == (0, 1)
assert next(it) == (1, 2)
