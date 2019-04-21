from testutils import assert_raises, assertRaises

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
assert set([1,2,3]).union([1,2,3,4,5]) == set([1,2,3,4,5])

assert set([1,2,3]) | set([4,5]) == set([1,2,3,4,5])
assert set([1,2,3]) | set([1,2,3,4,5]) == set([1,2,3,4,5])
assert_raises(TypeError, lambda: set([1,2,3]) | [1,2,3,4,5])

assert set([1,2,3]).intersection(set([1,2])) == set([1,2])
assert set([1,2,3]).intersection(set([5,6])) == set([])
assert set([1,2,3]).intersection([1,2]) == set([1,2])

assert set([1,2,3]) & set([4,5]) == set([])
assert set([1,2,3]) & set([1,2,3,4,5]) == set([1,2,3])
assert_raises(TypeError, lambda: set([1,2,3]) & [1,2,3,4,5])

assert set([1,2,3]).difference(set([1,2])) == set([3])
assert set([1,2,3]).difference(set([5,6])) == set([1,2,3])
assert set([1,2,3]).difference([1,2]) == set([3])

assert set([1,2,3]) - set([4,5]) == set([1,2,3])
assert set([1,2,3]) - set([1,2,3,4,5]) == set([])
assert_raises(TypeError, lambda: set([1,2,3]) - [1,2,3,4,5])

assert set([1,2,3]).symmetric_difference(set([1,2])) == set([3])
assert set([1,2,3]).symmetric_difference(set([5,6])) == set([1,2,3,5,6])
assert set([1,2,3]).symmetric_difference([1,2]) == set([3])

assert set([1,2,3]) ^ set([4,5]) == set([1,2,3,4,5])
assert set([1,2,3]) ^ set([1,2,3,4,5]) == set([4,5])
assert_raises(TypeError, lambda: set([1,2,3]) ^ [1,2,3,4,5])

assert set([1,2,3]).isdisjoint(set([5,6])) == True
assert set([1,2,3]).isdisjoint(set([2,5,6])) == False
assert set([1,2,3]).isdisjoint([5,6]) == True

assert_raises(TypeError, lambda: set([[]]))
assert_raises(TypeError, lambda: set().add([]))

a = set([1, 2, 3])
assert a.discard(1) is None
assert not 1 in a
assert a.discard(42) is None

a = set([1,2,3])
b = a.copy()
assert len(a) == 3
assert len(b) == 3
b.clear()
assert len(a) == 3
assert len(b) == 0

a = set([1,2])
b = a.pop()
assert b in [1,2]
c = a.pop()
assert (c in [1,2] and c != b) 
assert_raises(KeyError, lambda: a.pop())

a = set([1,2,3])
a.update([3,4,5])
assert a == set([1,2,3,4,5])
assert_raises(TypeError, lambda: a.update(1))

a = set([1,2,3])
b = set()
for e in a:
	assert e == 1 or e == 2 or e == 3
	b.add(e)
assert a == b

a = set([1,2,3])
a |= set([3,4,5])
assert a == set([1,2,3,4,5])
with assertRaises(TypeError):
	a |= 1
with assertRaises(TypeError):
	a |= [1,2,3]

a = set([1,2,3])
a.intersection_update([2,3,4,5])
assert a == set([2,3])
assert_raises(TypeError, lambda: a.intersection_update(1))

a = set([1,2,3])
a &= set([2,3,4,5])
assert a == set([2,3])
with assertRaises(TypeError):
	a &= 1
with assertRaises(TypeError):
	a &= [1,2,3]

a = set([1,2,3])
a.difference_update([3,4,5])
assert a == set([1,2])
assert_raises(TypeError, lambda: a.difference_update(1))

a = set([1,2,3])
a -= set([3,4,5])
assert a == set([1,2])
with assertRaises(TypeError):
	a -= 1
with assertRaises(TypeError):
	a -= [1,2,3]

a = set([1,2,3])
a.symmetric_difference_update([3,4,5])
assert a == set([1,2,4,5])
assert_raises(TypeError, lambda: a.difference_update(1))

a = set([1,2,3])
a ^= set([3,4,5])
assert a == set([1,2,4,5])
with assertRaises(TypeError):
	a ^= 1
with assertRaises(TypeError):
	a ^= [1,2,3]

# frozen set

assert frozenset([1,2]) == frozenset([1,2])
assert not frozenset([1,2,3]) == frozenset([1,2])

assert frozenset([1,2,3]) >= frozenset([1,2])
assert frozenset([1,2]) >= frozenset([1,2])
assert not frozenset([1,3]) >= frozenset([1,2])

assert frozenset([1,2,3]).issuperset(frozenset([1,2]))
assert frozenset([1,2]).issuperset(frozenset([1,2]))
assert not frozenset([1,3]).issuperset(frozenset([1,2]))

assert frozenset([1,2,3]) > frozenset([1,2])
assert not frozenset([1,2]) > frozenset([1,2])
assert not frozenset([1,3]) > frozenset([1,2])

assert frozenset([1,2]) <= frozenset([1,2,3])
assert frozenset([1,2]) <= frozenset([1,2])
assert not frozenset([1,3]) <= frozenset([1,2])

assert frozenset([1,2]).issubset(frozenset([1,2,3]))
assert frozenset([1,2]).issubset(frozenset([1,2]))
assert not frozenset([1,3]).issubset(frozenset([1,2]))

assert frozenset([1,2]) < frozenset([1,2,3])
assert not frozenset([1,2]) < frozenset([1,2])
assert not frozenset([1,3]) < frozenset([1,2])

a = frozenset([1, 2, 3])
assert len(a) == 3
b = a.copy()
assert b == a

assert frozenset([1,2,3]).union(frozenset([4,5])) == frozenset([1,2,3,4,5])
assert frozenset([1,2,3]).union(frozenset([1,2,3,4,5])) == frozenset([1,2,3,4,5])
assert frozenset([1,2,3]).union([1,2,3,4,5]) == frozenset([1,2,3,4,5])

assert frozenset([1,2,3]) | frozenset([4,5]) == frozenset([1,2,3,4,5])
assert frozenset([1,2,3]) | frozenset([1,2,3,4,5]) == frozenset([1,2,3,4,5])
assert_raises(TypeError, lambda: frozenset([1,2,3]) | [1,2,3,4,5])

assert frozenset([1,2,3]).intersection(frozenset([1,2])) == frozenset([1,2])
assert frozenset([1,2,3]).intersection(frozenset([5,6])) == frozenset([])
assert frozenset([1,2,3]).intersection([1,2]) == frozenset([1,2])

assert frozenset([1,2,3]) & frozenset([4,5]) == frozenset([])
assert frozenset([1,2,3]) & frozenset([1,2,3,4,5]) == frozenset([1,2,3])
assert_raises(TypeError, lambda: frozenset([1,2,3]) & [1,2,3,4,5])

assert frozenset([1,2,3]).difference(frozenset([1,2])) == frozenset([3])
assert frozenset([1,2,3]).difference(frozenset([5,6])) == frozenset([1,2,3])
assert frozenset([1,2,3]).difference([1,2]) == frozenset([3])

assert frozenset([1,2,3]) - frozenset([4,5]) == frozenset([1,2,3])
assert frozenset([1,2,3]) - frozenset([1,2,3,4,5]) == frozenset([])
assert_raises(TypeError, lambda: frozenset([1,2,3]) - [1,2,3,4,5])

assert frozenset([1,2,3]).symmetric_difference(frozenset([1,2])) == frozenset([3])
assert frozenset([1,2,3]).symmetric_difference(frozenset([5,6])) == frozenset([1,2,3,5,6])
assert frozenset([1,2,3]).symmetric_difference([1,2]) == frozenset([3])

assert frozenset([1,2,3]) ^ frozenset([4,5]) == frozenset([1,2,3,4,5])
assert frozenset([1,2,3]) ^ frozenset([1,2,3,4,5]) == frozenset([4,5])
assert_raises(TypeError, lambda: frozenset([1,2,3]) ^ [1,2,3,4,5])

assert frozenset([1,2,3]).isdisjoint(frozenset([5,6])) == True
assert frozenset([1,2,3]).isdisjoint(frozenset([2,5,6])) == False
assert frozenset([1,2,3]).isdisjoint([5,6]) == True

assert_raises(TypeError, lambda: frozenset([[]]))

a = frozenset([1,2,3])
b = set()
for e in a:
	assert e == 1 or e == 2 or e == 3
	b.add(e)
assert a == b

# set and frozen set
assert frozenset([1,2,3]).union(set([4,5])) == frozenset([1,2,3,4,5])
assert set([1,2,3]).union(frozenset([4,5])) == set([1,2,3,4,5])

assert frozenset([1,2,3]) | set([4,5]) == frozenset([1,2,3,4,5])
assert set([1,2,3]) | frozenset([4,5]) == set([1,2,3,4,5])

assert frozenset([1,2,3]).intersection(set([5,6])) == frozenset([])
assert set([1,2,3]).intersection(frozenset([5,6])) == set([])

assert frozenset([1,2,3]) & set([1,2,3,4,5]) == frozenset([1,2,3])
assert set([1,2,3]) & frozenset([1,2,3,4,5]) == set([1,2,3])

assert frozenset([1,2,3]).difference(set([5,6])) == frozenset([1,2,3])
assert set([1,2,3]).difference(frozenset([5,6])) == set([1,2,3])

assert frozenset([1,2,3]) - set([4,5]) == frozenset([1,2,3])
assert set([1,2,3]) - frozenset([4,5]) == frozenset([1,2,3])

assert frozenset([1,2,3]).symmetric_difference(set([1,2])) == frozenset([3])
assert set([1,2,3]).symmetric_difference(frozenset([1,2])) == set([3])

assert frozenset([1,2,3]) ^ set([4,5]) == frozenset([1,2,3,4,5])
assert set([1,2,3]) ^ frozenset([4,5]) == set([1,2,3,4,5])
