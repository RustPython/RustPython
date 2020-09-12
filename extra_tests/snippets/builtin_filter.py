assert list(filter(lambda x: ((x % 2) == 0), [0, 1, 2])) == [0, 2]

# None implies identity
assert list(filter(None, [0, 1, 2])) == [1, 2]

assert type(filter(None, [])) == filter


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


def predicate(x):
    if x == 0:
        raise StopIteration()
    return True


assert list(filter(predicate, [1, 2, 0, 4, 5])) == [1, 2]
