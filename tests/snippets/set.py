assert set([1,2]) == set([1,2])
assert not set([1,2,3]) == set([1,2])

assert set([1,2,3]) >= set([1,2])
assert set([1,2]) >= set([1,2])
assert not set([1,3]) >= set([1,2])

assert set([1,2,3]).issuperset(set([1,2]))
assert set([1,2]).issuperset(set([1,2]))
assert not set([1,3]).issuperset(set([1,2]))

assert set([1,2,3]) > set([1,2])
assert not set([1,2]) > set([1,2])
assert not set([1,3]) > set([1,2])

assert set([1,2]) <= set([1,2,3])
assert set([1,2]) <= set([1,2])
assert not set([1,3]) <= set([1,2])

assert set([1,2]).issubset(set([1,2,3]))
assert set([1,2]).issubset(set([1,2]))
assert not set([1,3]).issubset(set([1,2]))

assert set([1,2]) < set([1,2,3])
assert not set([1,2]) < set([1,2])
assert not set([1,3]) < set([1,2])

class Hashable(object):
    def __init__(self, obj):
        self.obj = obj

    def __repr__(self):
        return repr(self.obj)

    def __hash__(self):
        return id(self)


recursive = set()
recursive.add(Hashable(recursive))
assert repr(recursive) == "{set(...)}"

a = set([1, 2, 3])
assert len(a) == 3
a.clear()
assert len(a) == 0

assert set([1,2,3]).union(set([4,5])) == set([1,2,3,4,5])
assert set([1,2,3]).union(set([1,2,3,4,5])) == set([1,2,3,4,5])

assert set([1,2,3]) | set([4,5]) == set([1,2,3,4,5])
assert set([1,2,3]) | set([1,2,3,4,5]) == set([1,2,3,4,5])
