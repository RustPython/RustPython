assert list(enumerate(['a', 'b', 'c'])) == [(0, 'a'), (1, 'b'), (2, 'c')]

assert type(enumerate([])) == enumerate

assert list(enumerate(['a', 'b', 'c'], -100)) == [(-100, 'a'), (-99, 'b'), (-98, 'c')]
assert list(enumerate(['a', 'b', 'c'], 2**200)) == [(2**200, 'a'), (2**200 + 1, 'b'), (2**200 + 2, 'c')]

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
