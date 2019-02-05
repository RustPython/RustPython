assert list(filter(lambda x: ((x % 2) == 0), [0, 1, 2])) == [0, 2]

# None implies identity
assert list(filter(None, [0, 1, 2])) == [1, 2]


# test infinite iterator
class Counter(object):
    counter = 0

    def __next__(self):
        self.counter += 1
        return self.counter

    def __iter__(self):
        return self


it = filter(lambda x: ((x % 2) == 0), Counter())
assert next(it) == 2
assert next(it) == 4
