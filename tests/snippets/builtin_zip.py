assert list(zip(['a', 'b', 'c'], range(3), [9, 8, 7, 99])) == [('a', 0, 9), ('b', 1, 8), ('c', 2, 7)]

assert list(zip(['a', 'b', 'c'])) == [('a',), ('b',), ('c',)]
assert list(zip()) == []

assert list(zip(*zip(['a', 'b', 'c'], range(1, 4)))) == [('a', 'b', 'c'), (1, 2, 3)]


# test infinite iterator
class Counter(object):
    def __init__(self, counter=0):
        self.counter = counter

    def __next__(self):
        self.counter += 1
        return self.counter

    def __iter__(self):
        return self


it = zip(Counter(), Counter(3))
assert next(it) == (1, 4)
assert next(it) == (2, 5)
