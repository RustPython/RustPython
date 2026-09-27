from testutils import assert_raises

assert set([1, 2]) == set([1, 2])
assert not set([1, 2, 3]) == set([1, 2])

assert set([1, 2, 3]) >= set([1, 2])
assert set([1, 2]) >= set([1, 2])
assert not set([1, 3]) >= set([1, 2])

assert set([1, 2, 3]).issuperset(set([1, 2]))
assert set([1, 2]).issuperset(set([1, 2]))
assert not set([1, 3]).issuperset(set([1, 2]))

assert set([1, 2, 3]) > set([1, 2])
assert not set([1, 2]) > set([1, 2])
assert not set([1, 3]) > set([1, 2])

assert set([1, 2]) <= set([1, 2, 3])
assert set([1, 2]) <= set([1, 2])
assert not set([1, 3]) <= set([1, 2])

assert set([1, 2]).issubset(set([1, 2, 3]))
assert set([1, 2]).issubset(set([1, 2]))
assert not set([1, 3]).issubset(set([1, 2]))

assert set([1, 2]) < set([1, 2, 3])
assert not set([1, 2]) < set([1, 2])
assert not set([1, 3]) < set([1, 2])

assert (set() == []) is False
assert set().__eq__([]) == NotImplemented
assert_raises(
    TypeError,
    lambda: set() < [],
    _msg="'<' not supported between instances of 'set' and 'list'",
)
assert_raises(
    TypeError,
    lambda: set() <= [],
    _msg="'<=' not supported between instances of 'set' and 'list'",
)
assert_raises(
    TypeError,
    lambda: set() > [],
    _msg="'>' not supported between instances of 'set' and 'list'",
)
assert_raises(
    TypeError,
    lambda: set() >= [],
    _msg="'>=' not supported between instances of 'set' and 'list'",
)
assert set().issuperset([])
assert set().issubset([])
assert not set().issuperset([1, 2, 3])
assert set().issubset([1, 2])

assert (set() == 3) is False
assert set().__eq__(3) == NotImplemented
assert_raises(TypeError, lambda: set() < 3, _msg="'int' object is not iterable")
assert_raises(TypeError, lambda: set() <= 3, _msg="'int' object is not iterable")
assert_raises(TypeError, lambda: set() > 3, _msg="'int' object is not iterable")
assert_raises(TypeError, lambda: set() >= 3, _msg="'int' object is not iterable")
assert_raises(TypeError, set().issuperset, 3, _msg="'int' object is not iterable")
assert_raises(TypeError, set().issubset, 3, _msg="'int' object is not iterable")


class Hashable(object):
    def __init__(self, obj):
        self.obj = obj

    def __repr__(self):
        return repr(self.obj)

    def __hash__(self):
        return id(self)


assert repr(set()) == "set()"
assert repr(set([1, 2, 3])) == "{1, 2, 3}"

recursive = set()
recursive.add(Hashable(recursive))
assert repr(recursive) == "{set(...)}"


class S(set):
    pass


assert repr(S()) == "S()"
assert repr(S([1, 2, 3])) == "S({1, 2, 3})"

recursive = S()
recursive.add(Hashable(recursive))
assert repr(recursive) == "S({S(...)})"

a = set([1, 2, 3])
assert len(a) == 3
a.clear()
assert len(a) == 0

assert set([1, 2, 3]).union(set([4, 5])) == set([1, 2, 3, 4, 5])
assert set([1, 2, 3]).union(set([1, 2, 3, 4, 5])) == set([1, 2, 3, 4, 5])
assert set([1, 2, 3]).union([1, 2, 3, 4, 5]) == set([1, 2, 3, 4, 5])

assert set([1, 2, 3]) | set([4, 5]) == set([1, 2, 3, 4, 5])
assert set([1, 2, 3]) | set([1, 2, 3, 4, 5]) == set([1, 2, 3, 4, 5])
assert_raises(TypeError, lambda: set([1, 2, 3]) | [1, 2, 3, 4, 5])

assert set([1, 2, 3]).intersection(set([1, 2])) == set([1, 2])
assert set([1, 2, 3]).intersection(set([5, 6])) == set([])
assert set([1, 2, 3]).intersection([1, 2]) == set([1, 2])

assert set([1, 2, 3]) & set([4, 5]) == set([])
assert set([1, 2, 3]) & set([1, 2, 3, 4, 5]) == set([1, 2, 3])
assert_raises(TypeError, lambda: set([1, 2, 3]) & [1, 2, 3, 4, 5])

assert set([1, 2, 3]).difference(set([1, 2])) == set([3])
assert set([1, 2, 3]).difference(set([5, 6])) == set([1, 2, 3])
assert set([1, 2, 3]).difference([1, 2]) == set([3])

assert set([1, 2, 3]) - set([4, 5]) == set([1, 2, 3])
assert set([1, 2, 3]) - set([1, 2, 3, 4, 5]) == set([])
assert_raises(TypeError, lambda: set([1, 2, 3]) - [1, 2, 3, 4, 5])

assert set([1, 2]).__sub__(set([2, 3])) == set([1])
assert set([1, 2]).__rsub__(set([2, 3])) == set([3])

assert set([1, 2, 3]).symmetric_difference(set([1, 2])) == set([3])
assert set([1, 2, 3]).symmetric_difference(set([5, 6])) == set([1, 2, 3, 5, 6])
assert set([1, 2, 3]).symmetric_difference([1, 2]) == set([3])

assert set([1, 2, 3]) ^ set([4, 5]) == set([1, 2, 3, 4, 5])
assert set([1, 2, 3]) ^ set([1, 2, 3, 4, 5]) == set([4, 5])
assert_raises(TypeError, lambda: set([1, 2, 3]) ^ [1, 2, 3, 4, 5])

assert set([1, 2, 3]).isdisjoint(set([5, 6])) == True
assert set([1, 2, 3]).isdisjoint(set([2, 5, 6])) == False
assert set([1, 2, 3]).isdisjoint([5, 6]) == True

assert_raises(TypeError, lambda: set() & [])
assert_raises(TypeError, lambda: set() | [])
assert_raises(TypeError, lambda: set() ^ [])
assert_raises(TypeError, lambda: set() + [])
assert_raises(TypeError, lambda: set() - [])

assert_raises(TypeError, set, [[]])
assert_raises(TypeError, set().add, [])

a = set([1, 2, 3])
assert a.discard(1) is None
assert not 1 in a
assert a.discard(42) is None

a = set([1, 2, 3])
b = a.copy()
assert len(a) == 3
assert len(b) == 3
b.clear()
assert len(a) == 3
assert len(b) == 0

a = set([1, 2])
b = a.pop()
assert b in [1, 2]
c = a.pop()
assert c in [1, 2] and c != b
assert_raises(KeyError, lambda: a.pop())

a = set([1, 2, 3])
a.update([3, 4, 5])
assert a == set([1, 2, 3, 4, 5])
assert_raises(TypeError, lambda: a.update(1))

a = set([1, 2, 3])
b = set()
for e in a:
    assert e == 1 or e == 2 or e == 3
    b.add(e)
assert a == b

a = set([1, 2, 3])
a |= set([3, 4, 5])
assert a == set([1, 2, 3, 4, 5])
with assert_raises(TypeError):
    a |= 1
with assert_raises(TypeError):
    a |= [1, 2, 3]

a = set([1, 2, 3])
a.intersection_update([2, 3, 4, 5])
assert a == set([2, 3])
assert_raises(TypeError, lambda: a.intersection_update(1))

a = set([1, 2, 3])
a &= set([2, 3, 4, 5])
assert a == set([2, 3])
with assert_raises(TypeError):
    a &= 1
with assert_raises(TypeError):
    a &= [1, 2, 3]

a = set([1, 2, 3])
a &= a
assert a == set([1, 2, 3])

a = set([1, 2, 3])
a -= a
assert a == set()

a = set([1, 2, 3])
a ^= a
assert a == set()

a = set([1, 2, 3])
a.difference_update([3, 4, 5])
assert a == set([1, 2])
assert_raises(TypeError, lambda: a.difference_update(1))

a = set([1, 2, 3])
a -= set([3, 4, 5])
assert a == set([1, 2])
with assert_raises(TypeError):
    a -= 1
with assert_raises(TypeError):
    a -= [1, 2, 3]

a = set([1, 2, 3])
a.symmetric_difference_update([3, 4, 5])
assert a == set([1, 2, 4, 5])
assert_raises(TypeError, lambda: a.difference_update(1))

a = set([1, 2, 3])
a ^= set([3, 4, 5])
assert a == set([1, 2, 4, 5])
with assert_raises(TypeError):
    a ^= 1
with assert_raises(TypeError):
    a ^= [1, 2, 3]

a = set([1, 2, 3])
i = iter(a)
a.add(4)
with assert_raises(RuntimeError):
    next(i)
a.remove(4)
# TODO: Raises RuntimeError in CPython but we currently raise Runtime error.
# with assert_raises(StopIteration):
#     next(i)

# frozen set

assert frozenset([1, 2]) == frozenset([1, 2])
assert not frozenset([1, 2, 3]) == frozenset([1, 2])

assert frozenset([1, 2, 3]) >= frozenset([1, 2])
assert frozenset([1, 2]) >= frozenset([1, 2])
assert not frozenset([1, 3]) >= frozenset([1, 2])

assert frozenset([1, 2, 3]).issuperset(frozenset([1, 2]))
assert frozenset([1, 2]).issuperset(frozenset([1, 2]))
assert not frozenset([1, 3]).issuperset(frozenset([1, 2]))

assert frozenset([1, 2, 3]) > frozenset([1, 2])
assert not frozenset([1, 2]) > frozenset([1, 2])
assert not frozenset([1, 3]) > frozenset([1, 2])

assert frozenset([1, 2]) <= frozenset([1, 2, 3])
assert frozenset([1, 2]) <= frozenset([1, 2])
assert not frozenset([1, 3]) <= frozenset([1, 2])

assert frozenset([1, 2]).issubset(frozenset([1, 2, 3]))
assert frozenset([1, 2]).issubset(frozenset([1, 2]))
assert not frozenset([1, 3]).issubset(frozenset([1, 2]))

assert frozenset([1, 2]) < frozenset([1, 2, 3])
assert not frozenset([1, 2]) < frozenset([1, 2])
assert not frozenset([1, 3]) < frozenset([1, 2])

a = frozenset([1, 2, 3])
assert len(a) == 3
b = a.copy()
assert b == a

assert frozenset([1, 2, 3]).union(frozenset([4, 5])) == frozenset([1, 2, 3, 4, 5])
assert frozenset([1, 2, 3]).union(frozenset([1, 2, 3, 4, 5])) == frozenset(
    [1, 2, 3, 4, 5]
)
assert frozenset([1, 2, 3]).union([1, 2, 3, 4, 5]) == frozenset([1, 2, 3, 4, 5])

assert frozenset([1, 2, 3]) | frozenset([4, 5]) == frozenset([1, 2, 3, 4, 5])
assert frozenset([1, 2, 3]) | frozenset([1, 2, 3, 4, 5]) == frozenset([1, 2, 3, 4, 5])
assert_raises(TypeError, lambda: frozenset([1, 2, 3]) | [1, 2, 3, 4, 5])

assert frozenset([1, 2, 3]).intersection(frozenset([1, 2])) == frozenset([1, 2])
assert frozenset([1, 2, 3]).intersection(frozenset([5, 6])) == frozenset([])
assert frozenset([1, 2, 3]).intersection([1, 2]) == frozenset([1, 2])

assert frozenset([1, 2, 3]) & frozenset([4, 5]) == frozenset([])
assert frozenset([1, 2, 3]) & frozenset([1, 2, 3, 4, 5]) == frozenset([1, 2, 3])
assert_raises(TypeError, lambda: frozenset([1, 2, 3]) & [1, 2, 3, 4, 5])

assert frozenset([1, 2, 3]).difference(frozenset([1, 2])) == frozenset([3])
assert frozenset([1, 2, 3]).difference(frozenset([5, 6])) == frozenset([1, 2, 3])
assert frozenset([1, 2, 3]).difference([1, 2]) == frozenset([3])

assert frozenset([1, 2, 3]) - frozenset([4, 5]) == frozenset([1, 2, 3])
assert frozenset([1, 2, 3]) - frozenset([1, 2, 3, 4, 5]) == frozenset([])
assert_raises(TypeError, lambda: frozenset([1, 2, 3]) - [1, 2, 3, 4, 5])

assert frozenset([1, 2]).__sub__(frozenset([2, 3])) == frozenset([1])
assert frozenset([1, 2]).__rsub__(frozenset([2, 3])) == frozenset([3])

assert frozenset([1, 2, 3]).symmetric_difference(frozenset([1, 2])) == frozenset([3])
assert frozenset([1, 2, 3]).symmetric_difference(frozenset([5, 6])) == frozenset(
    [1, 2, 3, 5, 6]
)
assert frozenset([1, 2, 3]).symmetric_difference([1, 2]) == frozenset([3])

assert frozenset([1, 2, 3]) ^ frozenset([4, 5]) == frozenset([1, 2, 3, 4, 5])
assert frozenset([1, 2, 3]) ^ frozenset([1, 2, 3, 4, 5]) == frozenset([4, 5])
assert_raises(TypeError, lambda: frozenset([1, 2, 3]) ^ [1, 2, 3, 4, 5])

assert frozenset([1, 2, 3]).isdisjoint(frozenset([5, 6])) == True
assert frozenset([1, 2, 3]).isdisjoint(frozenset([2, 5, 6])) == False
assert frozenset([1, 2, 3]).isdisjoint([5, 6]) == True

assert_raises(TypeError, frozenset, [[]])

a = frozenset([1, 2, 3])
b = set()
for e in a:
    assert e == 1 or e == 2 or e == 3
    b.add(e)
assert a == b

# set and frozen set
assert frozenset([1, 2, 3]).union(set([4, 5])) == frozenset([1, 2, 3, 4, 5])
assert set([1, 2, 3]).union(frozenset([4, 5])) == set([1, 2, 3, 4, 5])

assert frozenset([1, 2, 3]) | set([4, 5]) == frozenset([1, 2, 3, 4, 5])
assert set([1, 2, 3]) | frozenset([4, 5]) == set([1, 2, 3, 4, 5])

assert frozenset([1, 2, 3]).intersection(set([5, 6])) == frozenset([])
assert set([1, 2, 3]).intersection(frozenset([5, 6])) == set([])

assert frozenset([1, 2, 3]) & set([1, 2, 3, 4, 5]) == frozenset([1, 2, 3])
assert set([1, 2, 3]) & frozenset([1, 2, 3, 4, 5]) == set([1, 2, 3])

assert frozenset([1, 2, 3]).difference(set([5, 6])) == frozenset([1, 2, 3])
assert set([1, 2, 3]).difference(frozenset([5, 6])) == set([1, 2, 3])

assert frozenset([1, 2, 3]) - set([4, 5]) == frozenset([1, 2, 3])
assert set([1, 2, 3]) - frozenset([4, 5]) == frozenset([1, 2, 3])

assert frozenset([1, 2]).__sub__(set([2, 3])) == frozenset([1])
assert frozenset([1, 2]).__rsub__(set([2, 3])) == set([3])
assert set([1, 2]).__sub__(frozenset([2, 3])) == set([1])
assert set([1, 2]).__rsub__(frozenset([2, 3])) == frozenset([3])

assert frozenset([1, 2, 3]).symmetric_difference(set([1, 2])) == frozenset([3])
assert set([1, 2, 3]).symmetric_difference(frozenset([1, 2])) == set([3])

assert frozenset([1, 2, 3]) ^ set([4, 5]) == frozenset([1, 2, 3, 4, 5])
assert set([1, 2, 3]) ^ frozenset([4, 5]) == set([1, 2, 3, 4, 5])


class A:
    def __hash__(self):
        return 1


class B:
    def __hash__(self):
        return 1


s = {1, A(), B()}
assert len(s) == 3

s = {True}
s.add(1.0)
assert str(s) == "{True}"


class EqObject:
    def __init__(self, eq):
        self.eq = eq

    def __eq__(self, other):
        return self.eq

    def __hash__(self):
        return bool(self.eq)


assert "x" == (EqObject("x") == EqObject("x"))
s = {EqObject("x")}
assert EqObject("x") in s
assert "[]" == (EqObject("[]") == EqObject("[]"))
s = {EqObject([])}
assert EqObject([]) not in s
x = object()
assert x == (EqObject(x) == EqObject(x))
s = {EqObject(x)}
assert EqObject(x) in s

assert set([1, 2]).__ne__(set())
assert not set([1, 2]).__ne__(set([2, 1]))
assert set().__ne__(1) == NotImplemented

assert frozenset([1, 2]).__ne__(set())
assert frozenset([1, 2]).__ne__(frozenset())
assert not frozenset([1, 2]).__ne__(set([2, 1]))
assert not frozenset([1, 2]).__ne__(frozenset([2, 1]))
assert frozenset().__ne__(1) == NotImplemented

empty_set = set()
non_empty_set = set([1, 2, 3])
set_from_literal = {1, 2, 3}

assert 1 in non_empty_set
assert 4 not in non_empty_set

assert 1 in set_from_literal
assert 4 not in set_from_literal

# TODO: Assert that empty aruguments raises exception.
non_empty_set.add("a")
assert "a" in non_empty_set

# TODO: Assert that empty arguments, or item not in set raises exception.
non_empty_set.remove(1)
assert 1 not in non_empty_set

# TODO: Assert that adding the same thing to a set once it's already there doesn't do anything.

assert repr(frozenset()) == "frozenset()"
assert repr(frozenset([1, 2, 3])) == "frozenset({1, 2, 3})"


class FS(frozenset):
    pass


assert repr(FS()) == "FS()"
assert repr(FS([1, 2, 3])) == "FS({1, 2, 3})"


class StoredHashKey:
    def __init__(self, value):
        self.value = value
        self.hash_enabled = True
        self.hash_count = 0

    def __hash__(self):
        self.hash_count += 1
        assert self.hash_enabled, "set operation recomputed a stored hash"
        return self.value


for left_type in (set, frozenset):
    for right_type in (set, frozenset):
        keys = [StoredHashKey(i) for i in range(3)]
        small = left_type(keys[:1])
        equal = right_type(keys[:1])
        large = right_type(keys[:2])
        separate = right_type(keys[2:])
        for key in keys:
            key.hash_enabled = False
        assert small == equal
        assert not (small != equal)
        assert small < large
        assert small <= large
        assert large > small
        assert large >= small
        assert small.issubset(large)
        assert large.issuperset(small)
        assert small.isdisjoint(separate)
        assert not small.isdisjoint(large)


for set_type in (set, frozenset):

    class IteratorOverride(set_type):
        def __iter__(self):
            raise RuntimeError("overridden iterator")

    subclass = IteratorOverride([1, 2])
    assert set_type([1]).issubset(subclass)
    assert set_type([1, 2, 3]).issuperset(subclass)
    assert set_type([1]).intersection(subclass) == {1}
    assert not subclass.isdisjoint(subclass)
    empty_subclass = IteratorOverride()
    assert empty_subclass.isdisjoint(empty_subclass)
    assert_raises(RuntimeError, set_type([3]).isdisjoint, subclass)


# Intersections retain the key from the smaller operand, or RHS on a tie.
small_key = float("1")
small = {small_key}
large = {1, 2}
assert next(iter(small & large)) is small_key
assert next(iter(large & small)) is small_key
assert next(iter({1} & small)) is small_key
result = large.intersection(small)
result.clear()
assert large == {1, 2}
assert small == {small_key}


def clear_during_intersection():
    live_source.clear()
    yield 1


live_source = {1, 2}
assert live_source.intersection(clear_during_intersection()) == set()


class DirectionalKey:
    def __init__(self, equal):
        self.equal = equal

    def __hash__(self):
        return 17

    def __eq__(self, other):
        return self.equal


small_key = DirectionalKey(False)
large_key = DirectionalKey(True)
small = {small_key}
large = {large_key, 12345}
equal_size = {large_key}
assert next(iter(small & large)) is small_key
assert next(iter(large & small)) is small_key
assert not small.isdisjoint(large)
assert not large.isdisjoint(small)
assert small.issubset(large)
assert large.issuperset(small)
assert small == equal_size
assert equal_size != small
assert not small.issubset([large_key])


def matched_then_error():
    yield 1
    raise ValueError("iterator consumed after matching")


for set_type in (set, frozenset):
    assert set_type([1]).intersection(matched_then_error()) == {1}
    assert set_type([1]).issubset(matched_then_error())
    assert_raises(ValueError, set_type([1, 2]).intersection, matched_then_error())
    assert_raises(ValueError, set_type([1, 2]).issubset, matched_then_error())
    assert_raises(ValueError, set_type().intersection, matched_then_error())
    assert_raises(TypeError, set_type().intersection, [set()])

key = StoredHashKey(4)
source = {key}
key.hash_count = 0
assert len(source.intersection([key])) == 1
assert key.hash_count == 1
dictionary = {key: None}
key.hash_enabled = False
assert_raises(AssertionError, source.intersection, dictionary)


# Native operations reuse hashes stored when an element enters the collection.
class RemainingHashKey:
    blocked = False

    def __init__(self, value):
        self.value = value

    def __hash__(self):
        assert not type(self).blocked, "stored key was hashed again"
        return 7

    def __eq__(self, other):
        return isinstance(other, RemainingHashKey) and self.value == other.value


class NativeSetSource(set):
    def __iter__(self):
        raise AssertionError("native set operation called __iter__")


class NativeFrozenSetSource(frozenset):
    def __iter__(self):
        raise AssertionError("native set operation called __iter__")


stored_keys = [RemainingHashKey(1), RemainingHashKey(2)]
stored_frozen = frozenset(stored_keys)
stored_frozen_reversed = frozenset(reversed(stored_keys))
stored_sources = [
    source_type(stored_keys)
    for source_type in (set, frozenset, NativeSetSource, NativeFrozenSetSource)
]
RemainingHashKey.blocked = True
stored_hash = hash(stored_frozen)
assert stored_hash == hash(stored_frozen_reversed)
assert stored_hash == hash(stored_frozen)
for stored_source in stored_sources:
    stored_target = set()
    assert stored_target.__ior__(stored_source) is stored_target
    assert len(stored_target) == 2
    assert {item.value for item in stored_target} == {1, 2}
    stored_target |= stored_target
    assert len(stored_target) == 2
RemainingHashKey.blocked = False


# Difference folds and reflected subtraction leave both operands unchanged.
for left_type in (set, frozenset):
    for right_type in (set, frozenset):
        left = left_type([1, 2, 3])
        right = right_type([2])
        assert left.difference(right, [3]) == {1}
        assert right.__rsub__(left) == {1, 3}
        assert left == {1, 2, 3}
        assert right == {2}


# The temporary set deduplicates input without hashing its keys a second time.
for set_type in (set, frozenset):
    present = StoredHashKey(10)
    added = StoredHashKey(11)
    source = set_type([present])
    present.hash_count = 0
    result = source.symmetric_difference([present, added, added])
    assert list(result) == [added]
    assert list(source) == [present]
    assert (present.hash_count, added.hash_count) == (1, 2)
    if set_type is set:
        present.hash_count = added.hash_count = 0
        source.symmetric_difference_update([present, added, added])
        assert list(source) == [added]
        assert (present.hash_count, added.hash_count) == (1, 2)


# Streaming removal retains progress if the input iterator later raises.
source = {0, 1, 2}
assert_raises(ValueError, source.difference_update, matched_then_error())
assert source == {0, 2}
source = {1, 2}
assert_raises(RuntimeError, source.difference_update, iter(source))
assert len(source) == 1
source.difference_update(source)
assert source == set()
