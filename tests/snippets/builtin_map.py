a = list(map(str, [1, 2, 3]))
assert a == ['1', '2', '3']


b = list(map(lambda x, y: x + y, [1, 2, 4], [3, 5]))
assert b == [4, 7]


# test infinite iterator
class Counter(object):
    counter = 0

    def __next__(self):
        self.counter += 1
        return self.counter

    def __iter__(self):
        return self


it = map(lambda x: x+1, Counter())
assert next(it) == 2
assert next(it) == 3
