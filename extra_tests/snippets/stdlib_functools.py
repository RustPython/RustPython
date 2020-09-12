from functools import reduce
from testutils import assert_raises

class Squares:
    def __init__(self, max):
        self.max = max
        self.sofar = []

    def __len__(self):
        return len(self.sofar)

    def __getitem__(self, i):
        if not 0 <= i < self.max: raise IndexError
        n = len(self.sofar)
        while n <= i:
            self.sofar.append(n*n)
            n += 1
        return self.sofar[i]

def add(a, b):
    return a + b

assert reduce(add, ['a', 'b', 'c']) == 'abc'
assert reduce(add, ['a', 'b', 'c'], str(42)) == '42abc'
assert reduce(add, [['a', 'c'], [], ['d', 'w']], []) == ['a','c','d','w']
assert reduce(add, [['a', 'c'], [], ['d', 'w']], []) == ['a','c','d','w']
assert reduce(lambda x, y: x*y, range(2, 21), 1) == 2432902008176640000
assert reduce(add, Squares(10)) == 285
assert reduce(add, Squares(10), 0) == 285
assert reduce(add, Squares(0), 0) == 0
assert reduce(42, "1") == "1"
assert reduce(42, "", "1") == "1"

with assert_raises(TypeError):
    reduce()

with assert_raises(TypeError):
    reduce(42, 42)

with assert_raises(TypeError):
    reduce(42, 42, 42)

class TestFailingIter:
    def __iter__(self):
        raise RuntimeError

with assert_raises(RuntimeError):
    reduce(add, TestFailingIter())

assert reduce(add, [], None) == None
assert reduce(add, [], 42) == 42

class BadSeq:
    def __getitem__(self, index):
        raise ValueError
with assert_raises(ValueError):
    reduce(42, BadSeq())

# Test reduce()'s use of iterators.
class SequenceClass:
    def __init__(self, n):
        self.n = n
    def __getitem__(self, i):
        if 0 <= i < self.n:
            return i
        else:
            raise IndexError

assert reduce(add, SequenceClass(5)) == 10
assert reduce(add, SequenceClass(5), 42) == 52
with assert_raises(TypeError):
    reduce(add, SequenceClass(0))

assert reduce(add, SequenceClass(0), 42) == 42
assert reduce(add, SequenceClass(1)) == 0
assert reduce(add, SequenceClass(1), 42) == 42

d = {"one": 1, "two": 2, "three": 3}
assert reduce(add, d) == "".join(d.keys())
