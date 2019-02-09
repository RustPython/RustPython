
empty_set = set()
non_empty_set = set([1,2,3])
set_from_literal = {1,2,3}

assert 1 in non_empty_set
assert 4 not in non_empty_set

assert 1 in set_from_literal
assert 4 not in set_from_literal

# TODO: Assert that empty aruguments raises exception.
non_empty_set.add('a')
assert 'a' in non_empty_set

# TODO: Assert that empty arguments, or item not in set raises exception.
non_empty_set.remove(1)
assert 1 not in non_empty_set

# TODO: Assert that adding the same thing to a set once it's already there doesn't do anything.
