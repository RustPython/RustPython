assert set([1,2,2,3]) == set([1,2,3])

assert len(set([1,2,3])) == 3
assert len(set([1,2,2,3])) == 3

try:
	set([[]])
except TypeError:
    pass
else:
    assert False, "TypeError was not raised"


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

assert set([1,2,3]).union(set([4,5])) == set([1,2,3,4,5])
assert set([1,2,3]).union(set([1,2,3,4,5])) == set([1,2,3,4,5])

assert set([1,2,3]) | set([4,5]) == set([1,2,3,4,5])
assert set([1,2,3]) | set([1,2,3,4,5]) == set([1,2,3,4,5])

assert set([1,2,3]).intersection(set([1,2])) == set([1,2])
assert set([1,2,3]).intersection(set([5,6])) == set([])

assert set([1,2,3]) & set([4,5]) == set([])
assert set([1,2,3]) & set([1,2,3,4,5]) == set([1,2,3])

assert set([1,2,3]).difference(set([1,2])) == set([3])
assert set([1,2,3]).difference(set([5,6])) == set([1,2,3])

assert set([1,2,3]) - set([4,5]) == set([1,2,3])
assert set([1,2,3]) - set([1,2,3,4,5]) == set([])

assert set([1,2,3]).symmetric_difference(set([1,2])) == set([3])
assert set([1,2,3]).symmetric_difference(set([5,6])) == set([1,2,3,5,6])

assert set([1,2,3]) ^ set([4,5]) == set([1,2,3,4,5])
assert set([1,2,3]) ^ set([1,2,3,4,5]) == set([4,5])
