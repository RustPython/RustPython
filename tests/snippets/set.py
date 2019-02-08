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
