from testutils import assert_raises

assert len(dict()) == 0

assert len({}) == 0
assert len({"a": "b"}) == 1
assert len({"a": "b", "b": 1}) == 2
assert len({"a": "b", "b": 1, "a" + "b": 2 * 2}) == 3

d = {}
d["a"] = d
assert repr(d) == "{'a': {...}}"

assert {"a": 123}.get("a") == 123
assert {"a": 123}.get("b") == None
assert {"a": 123}.get("b", 456) == 456

d = {"a": 123, "b": 456}
assert list(reversed(d)) == ["b", "a"]
assert list(reversed(d.keys())) == ["b", "a"]
assert list(reversed(d.values())) == [456, 123]
assert list(reversed(d.items())) == [("b", 456), ("a", 123)]
with assert_raises(StopIteration):
    dict_reversed = reversed(d)
    for _ in range(len(d) + 1):
        next(dict_reversed)
assert "dict" in dict().__doc__


d = {"a": 123, "b": 456}
assert 1 not in d.items()
assert "a" not in d.items()
assert "a", 123 not in d.items()
assert () not in d.items()
assert (1) not in d.items()
assert ("a") not in d.items()
assert ("a", 123) in d.items()
assert ("b", 456) in d.items()
assert ("a", 123, 3) not in d.items()
assert ("a", 123, "b", 456) not in d.items()

d = {1: 10, "a": "ABC", (3, 4): 5}
assert 1 in d.keys()
assert (1) in d.keys()
assert "a" in d.keys()
assert (3, 4) in d.keys()
assert () not in d.keys()
assert 10 not in d.keys()
assert (1, 10) not in d.keys()
assert "abc" not in d.keys()
assert ((3, 4), 5) not in d.keys()

d1 = {"a": 1, "b": 2}
d2 = {"c": 3, "d": 4}
assert d1.items().isdisjoint(d2.items())
assert d1.keys().isdisjoint(d2.keys())
d2 = {"b": 3, "d": 4}
assert d1.items().isdisjoint(d2.items())
assert not d1.keys().isdisjoint(d2.keys())
d2 = {"c": 2, "d": 4}
assert d1.items().isdisjoint(d2.items())
assert d1.keys().isdisjoint(d2.keys())
d2 = {"b": 2, "d": 4}
assert not d1.items().isdisjoint(d2.items())
assert not d1.keys().isdisjoint(d2.keys())


assert dict(a=2, b=3) == {"a": 2, "b": 3}
assert dict({"a": 2, "b": 3}, b=4) == {"a": 2, "b": 4}
assert dict([("a", 2), ("b", 3)]) == {"a": 2, "b": 3}

assert {} == {}
assert not {"a": 2} == {}
assert not {} == {"a": 2}
assert not {"b": 2} == {"a": 2}
assert not {"a": 4} == {"a": 2}
assert {"a": 2} == {"a": 2}

nan = float("nan")
assert {"a": nan} == {"a": nan}

a = {"g": 5}
b = {"a": a, "d": 9}
c = dict(b)
c["d"] = 3
c["a"]["g"] = 2
assert a == {"g": 2}
assert b == {"a": a, "d": 9}

a.clear()
assert len(a) == 0

a = {"a": 5, "b": 6}
res = set()
for value in a.values():
    res.add(value)
assert res == set([5, 6])

count = 0
for key, value in a.items():
    assert a[key] == value
    count += 1
assert count == len(a)

res = set()
for key in a.keys():
    res.add(key)
assert res == set(["a", "b"])

# Deleted values are correctly skipped over:
x = {"a": 1, "b": 2, "c": 3, "d": 3}
del x["c"]
it = iter(x.items())
assert ("a", 1) == next(it)
assert ("b", 2) == next(it)
assert ("d", 3) == next(it)
with assert_raises(StopIteration):
    next(it)

with assert_raises(KeyError) as cm:
    del x[10]
assert cm.exception.args[0] == 10

# Iterating a dictionary is just its keys:
assert ["a", "b", "d"] == list(x)

# Iterating view captures dictionary when iterated.
data = {1: 2, 3: 4}
items = data.items()
assert list(items) == [(1, 2), (3, 4)]
data[5] = 6
assert list(items) == [(1, 2), (3, 4), (5, 6)]

# Values can be changed during iteration.
data = {1: 2, 3: 4}
items = iter(data.items())
assert (1, 2) == next(items)
data[3] = "changed"
assert (3, "changed") == next(items)

# But we can't add or delete items during iteration.
d = {}
a = iter(d.items())
d["a"] = 2
b = iter(d.items())
assert ("a", 2) == next(b)
with assert_raises(RuntimeError):
    next(a)
del d["a"]
with assert_raises(RuntimeError):
    next(b)

# View isn't itself an iterator.
with assert_raises(TypeError):
    next(data.keys())

assert len(data.keys()) == 2

x = {}
x[1] = 1
assert x[1] == 1

x[7] = 7
x[2] = 2
x[(5, 6)] = 5

with assert_raises(TypeError):
    x[[]]  # Unhashable type.

x["here"] = "here"
assert x.get("not here", "default") == "default"
assert x.get("here", "default") == "here"
assert x.get("not here") == None


class LengthDict(dict):
    def __getitem__(self, k):
        return len(k)


x = LengthDict()
assert type(x) == LengthDict
assert x["word"] == 4
assert x.get("word") is None

assert 5 == eval("a + word", LengthDict())


class Squares(dict):
    def __missing__(self, k):
        v = k * k
        self[k] = v
        return v


x = Squares()
assert x[-5] == 25


# An object that hashes to the same value always, and compares equal if any its values match.
class Hashable(object):
    def __init__(self, *args):
        self.values = args

    def __hash__(self):
        return 1

    def __eq__(self, other):
        for x in self.values:
            for y in other.values:
                if x == y:
                    return True
        return False


x = {}
x[Hashable(1, 2)] = 8

assert x[Hashable(1, 2)] == 8
assert x[Hashable(3, 1)] == 8

x[Hashable(8)] = 19
x[Hashable(19, 8)] = 1
assert x[Hashable(8)] == 1
assert len(x) == 2

assert list({"a": 2, "b": 10}) == ["a", "b"]
x = {}
x["a"] = 2
x["b"] = 10
assert list(x) == ["a", "b"]

y = x.copy()
x["c"] = 12
assert y == {"a": 2, "b": 10}

y.update({"c": 19, "d": -1, "b": 12})
assert y == {"a": 2, "b": 12, "c": 19, "d": -1}

y.update(y)
assert y == {"a": 2, "b": 12, "c": 19, "d": -1}  # hasn't changed

# KeyError has object that used as key as an .args[0]
with assert_raises(KeyError) as cm:
    x["not here"]
assert cm.exception.args[0] == "not here"
with assert_raises(KeyError) as cm:
    x.pop("not here")
assert cm.exception.args[0] == "not here"

with assert_raises(KeyError) as cm:
    x[10]
assert cm.exception.args[0] == 10
with assert_raises(KeyError) as cm:
    x.pop(10)
assert cm.exception.args[0] == 10


class MyClass:
    pass


obj = MyClass()

with assert_raises(KeyError) as cm:
    x[obj]
assert cm.exception.args[0] == obj
with assert_raises(KeyError) as cm:
    x.pop(obj)
assert cm.exception.args[0] == obj

x = {1: "a", "1": None}
assert x.pop(1) == "a"
assert x.pop("1") is None
assert x == {}

x = {1: "a"}
assert (1, "a") == x.popitem()
assert x == {}
with assert_raises(KeyError) as cm:
    x.popitem()
assert cm.exception.args == ("popitem(): dictionary is empty",)

x = {"a": 4}
assert 4 == x.setdefault("a", 0)
assert x["a"] == 4
assert 0 == x.setdefault("b", 0)
assert x["b"] == 0
assert None == x.setdefault("c")
assert x["c"] is None

assert {1: None, "b": None} == dict.fromkeys([1, "b"])
assert {1: 0, "b": 0} == dict.fromkeys([1, "b"], 0)

for source in ({i: -1 for i in range(128)}, set(range(128)), frozenset(range(128))):
    shared_value = []
    result = dict.fromkeys(source, shared_value)
    assert list(result) == list(source)
    assert all(value is shared_value for value in result.values())


class UnsizedFromKeys:
    def __iter__(self):
        return iter((1, 1, 2))

    def __len__(self):
        raise AssertionError("fromkeys must not request a length hint")

    __length_hint__ = __len__


assert dict.fromkeys(UnsizedFromKeys()) == {1: None, 2: None}

x = {"a": 1, "b": 1, "c": 1}
y = {"b": 2, "c": 2, "d": 2}
z = {"c": 3, "d": 3, "e": 3}

w = {1: 1, **x, 2: 2, **y, 3: 3, **z, 4: 4}
assert w == {
    1: 1,
    "a": 1,
    "b": 2,
    "c": 3,
    2: 2,
    "d": 3,
    3: 3,
    "e": 3,
    4: 4,
}  # not in cpython test suite

assert str({True: True, 1.0: 1.0}) == str({True: 1.0})


class A:
    def __hash__(self):
        return 1

    def __eq__(self, other):
        return isinstance(other, A)


class B:
    def __hash__(self):
        return 1

    def __eq__(self, other):
        return isinstance(other, B)


s = {1: 0, A(): 1, B(): 2}
assert len(s) == 3
assert s[1] == 0
assert s[A()] == 1
assert s[B()] == 2

# Test dict usage in set with star expressions!
a = {"bla": 2}
b = {"c": 44, "bla": 332, "d": 6}
x = ["bla", "c", "d", "f"]
c = {*a, *b, *x}
# print(c, type(c))
assert isinstance(c, set)
assert c == {"bla", "c", "d", "f"}

assert not {}.__ne__({})
assert {}.__ne__({"a": "b"})
assert {}.__ne__(1) == NotImplemented

it = iter({0: 1, 2: 3, 4: 5, 6: 7})
assert it.__length_hint__() == 4
next(it)
assert it.__length_hint__() == 3
next(it)
assert it.__length_hint__() == 2
next(it)
assert it.__length_hint__() == 1
next(it)
assert it.__length_hint__() == 0
assert_raises(StopIteration, next, it)
assert it.__length_hint__() == 0

# Test dictionary unpacking with non-mapping objects
# This should raise TypeError for non-mapping objects
with assert_raises(TypeError) as cm:
    {**[1, 2]}
assert "'list' object is not a mapping" in str(cm.exception)

with assert_raises(TypeError) as cm:
    {**[[1, 2], [3, 4]]}
assert "'list' object is not a mapping" in str(cm.exception)

with assert_raises(TypeError) as cm:
    {**"string"}
assert "'str' object is not a mapping" in str(cm.exception)

with assert_raises(TypeError) as cm:
    {**(1, 2, 3)}
assert "'tuple' object is not a mapping" in str(cm.exception)

# Test that valid mappings still work
assert {**{"a": 1}, **{"b": 2}} == {"a": 1, "b": 2}

# Test OrderedDict unpacking preserves order
import collections

od = collections.OrderedDict([("a", 1), ("b", 2)])
od.move_to_end("a")  # Move 'a' to end: ['b', 'a']
expected_order = list(od.items())  # [('b', 2), ('a', 1)]


def test_func(**kwargs):
    return kwargs


result = test_func(**od)
assert list(result.items()) == expected_order, (
    f"Expected {expected_order}, got {list(result.items())}"
)

# Test multiple OrderedDict unpacking
od1 = collections.OrderedDict([("x", 10), ("y", 20)])
od2 = collections.OrderedDict([("z", 30), ("w", 40)])
od2.move_to_end("z")  # Move 'z' to end: ['w', 'z']

result = test_func(**od1, **od2)
# Should preserve order: x, y, w, z
expected_keys = ["x", "y", "w", "z"]
assert list(result.keys()) == expected_keys, (
    f"Expected {expected_keys}, got {list(result.keys())}"
)


def check_view_comparisons(view, other, expected):
    assert (
        view == other,
        view != other,
        view < other,
        view <= other,
        view > other,
        view >= other,
    ) == expected
    assert (
        other == view,
        other != view,
        other > view,
        other >= view,
        other < view,
        other <= view,
    ) == expected


# Dictionary views support the same comparisons with mutable and frozen sets.
config = {"host": "localhost", "port": 8080}
for view in (config.keys(), config.items()):
    for set_type in (set, frozenset):
        schema = set_type(view)
        check_view_comparisons(view, schema, (True, False, False, True, False, True))
        smaller_schema = schema - {next(iter(schema))}
        check_view_comparisons(
            view, smaller_schema, (False, True, False, False, True, True)
        )
        larger_schema = schema | {("extra",)}
        check_view_comparisons(
            view, larger_schema, (False, True, True, True, False, False)
        )
        check_view_comparisons(
            view, set_type({("other",)}), (False, True, False, False, False, False)
        )

# Mixed keys/items views compare their members regardless of insertion order.
indexed_settings = {("port", 8080): None, ("host", "localhost"): None}
check_view_comparisons(
    config.items(), indexed_settings.keys(), (True, False, False, True, False, True)
)
del indexed_settings[("port", 8080)]
check_view_comparisons(
    config.items(), indexed_settings.keys(), (False, True, False, False, True, True)
)

# Testing for common settings does not require hashable values.
left_settings = {"ports": [80, 443], "hosts": ["localhost"]}
right_settings = {"ports": [80, 443], "timeout": 30}
assert not left_settings.items().isdisjoint(right_settings.items())
assert not right_settings.items().isdisjoint(left_settings.items())
assert not left_settings.items().isdisjoint([("ports", [80, 443])])
assert left_settings.items().isdisjoint({"ports": [8080]}.items())
assert left_settings.items().isdisjoint(())
assert left_settings.items().isdisjoint({"timeout": 30}.items())
assert left_settings.items() != frozenset()
assert left_settings.items() >= frozenset()
assert not left_settings.items() <= frozenset()
settings_view = left_settings.items()
assert not settings_view.isdisjoint(settings_view)
assert {}.items().isdisjoint({}.items())


def settings_then_error(setting):
    yield setting
    raise ValueError("remaining settings unavailable")


# A match stops consumption, while an error before a match still propagates.
assert not config.keys().isdisjoint(settings_then_error("host"))
assert not left_settings.items().isdisjoint(settings_then_error(("ports", [80, 443])))
with assert_raises(ValueError):
    config.keys().isdisjoint(settings_then_error("missing"))
with assert_raises(TypeError):
    config.keys().isdisjoint(42)


class ViewMembershipKey:
    hash_calls = 0

    def __hash__(self):
        self.hash_calls += 1
        return 42


# Successful item membership needs one dictionary lookup.
membership_key = ViewMembershipKey()
membership_dict = {membership_key: [1, 2]}
membership_key.hash_calls = 0
assert (membership_key, [1, 2]) in membership_dict.items()
assert membership_key.hash_calls == 1

for set_type in (set, frozenset):

    class ViewSetSubclass(set_type):
        events = []

        def __iter__(self):
            self.events.append("iter")
            return super().__iter__()

        def __contains__(self, item):
            self.events.append("contains")
            return super().__contains__(item)

    # View operations preserve a set subclass's iteration and membership hooks.
    schema = ViewSetSubclass({"host", "port"})
    schema.events.clear()
    assert config.keys() == schema
    assert schema.events == ["contains", "contains"]
    schema.events.clear()
    assert config.keys() >= schema
    assert schema.events == ["iter"]
    schema.events.clear()
    assert not config.keys().isdisjoint(schema)
    assert schema.events == ["iter"]
    larger_schema = ViewSetSubclass({"host", "port", "timeout"})
    schema.events.clear()
    assert not config.keys().isdisjoint(larger_schema)
    assert schema.events == ["contains"]

# An empty view must still consume arbitrary iterables and validate their keys.
with assert_raises(TypeError):
    {}.keys().isdisjoint([[]])
with assert_raises(ValueError):
    {}.keys().isdisjoint(settings_then_error("missing"))

for set_type in (set, frozenset):

    class ClearingLengthSet(set_type):
        def __len__(self):
            changing_config.clear()
            return 1

        def __iter__(self):
            raise ValueError("settings unavailable")

    # The view's size is captured before calling the other operand's __len__.
    changing_config = {"host": "localhost", "port": 8080}
    with assert_raises(ValueError):
        changing_config.keys().isdisjoint(ClearingLengthSet({"host"}))
    assert changing_config == {}


class ItemLookupDict(dict):
    def __getitem__(self, key):
        raise AssertionError("item membership called __getitem__")

    def __missing__(self, key):
        raise AssertionError("item membership called __missing__")


# Item views inspect dictionary storage without invoking subclass lookup hooks.
lookup_items = ItemLookupDict(ports=[80, 443]).items()
assert ("ports", [80, 443]) in lookup_items
assert ("ports", [8080]) not in lookup_items
assert ("missing", []) not in lookup_items


class LookupOrderKey:
    calls = []

    def __init__(self, name, equal):
        self.name = name
        self.equal = equal

    def __hash__(self):
        return 17

    def __eq__(self, other):
        self.calls.append(self.name)
        return self.equal


stored = LookupOrderKey("stored", True)
lookup = LookupOrderKey("lookup", False)
mapping = {stored: 1}
assert mapping[lookup] == 1
mapping[lookup] = 2
assert len(mapping) == 1
assert next(iter(mapping)) is stored
del mapping[lookup]
assert not mapping
assert LookupOrderKey.calls == ["stored", "stored", "stored"]

LookupOrderKey.calls.clear()
stored = LookupOrderKey("stored", NotImplemented)
lookup = LookupOrderKey("lookup", True)
assert {stored: 1}[lookup] == 1
assert LookupOrderKey.calls == ["stored", "lookup"]


class LookupOrderSubclass(LookupOrderKey):
    __hash__ = LookupOrderKey.__hash__

    def __eq__(self, other):
        return super().__eq__(other)


LookupOrderKey.calls.clear()
stored = LookupOrderKey("stored", False)
lookup = LookupOrderSubclass("lookup", True)
assert {stored: 1}[lookup] == 1
assert LookupOrderKey.calls == ["lookup"]


class MergeHashKey:
    hash_disabled = False

    def __init__(self, value):
        self.value = value

    def __hash__(self):
        assert not self.hash_disabled, "dictionary merge rehashed a stored key"
        return 42

    def __eq__(self, other):
        if not isinstance(other, MergeHashKey):
            return NotImplemented
        return self.value == other.value


def merge_with_update(source):
    result = {}
    result.update(source)
    return result


def merge_with_ior(source):
    result = {}
    result |= source
    return result


# Exact dictionary merges reuse hashes, including colliding keys and holes.
merge_keys = [MergeHashKey(i) for i in range(3)]
merge_source = dict(zip(merge_keys, ("first", "removed", "last")))
del merge_source[merge_keys[1]]
existing_merge_key = MergeHashKey(2)
merge_target = {existing_merge_key: "old"}
MergeHashKey.hash_disabled = True
for merge in (
    dict,
    merge_with_update,
    merge_with_ior,
    lambda source: {} | source,
    lambda source: dict.__ror__(source, {}),
    lambda source: {**source},
):
    assert list(merge(merge_source).items()) == list(merge_source.items())

# Overwriting a matching key keeps its identity and position.
merge_target.update(merge_source)
assert next(iter(merge_target)) is existing_merge_key
assert list(merge_target.values()) == ["last", "first"]
merge_source.update(merge_source)
merge_source |= merge_source
assert list(merge_source.values()) == ["first", "last"]
MergeHashKey.hash_disabled = False


# Compact copies preserve order, key identity and stored hashes after deletions.
copy_keys = [MergeHashKey(i) for i in range(32)]
copy_source = {key: key.value for key in copy_keys}
for key in copy_keys[:-3]:
    del copy_source[key]
MergeHashKey.hash_disabled = True
copy_result = copy_source.copy()
assert list(copy_result.values()) == [29, 30, 31]
assert all(actual is expected for actual, expected in zip(copy_result, copy_keys[-3:]))
copy_result.clear()
assert len(copy_source) == 3
MergeHashKey.hash_disabled = False


class MergeMapping(dict):
    def __iter__(self):
        return iter(("virtual",))

    def keys(self):
        return ["virtual"]

    def __getitem__(self, key):
        assert key == "virtual"
        return 42


# Generic mappings retain their lookup hooks instead of exposing dict storage.
for merge in (dict, merge_with_update, merge_with_ior):
    assert merge(MergeMapping(stored=0)) == {"virtual": 42}

# Test hashability of dict and OrderedDict views
import collections.abc

d = {"a": 1, "b": 2}
assert type(d.keys()).__hash__ is None
assert type(d.items()).__hash__ is None
assert type(d.values()).__hash__ is not None
assert not isinstance(d.keys(), collections.abc.Hashable)
assert not isinstance(d.items(), collections.abc.Hashable)
with assert_raises(TypeError):
    hash(d.keys())
with assert_raises(TypeError):
    hash(d.items())

od = collections.OrderedDict([("a", 1)])
assert type(od.keys()).__hash__ is None
assert type(od.items()).__hash__ is None
assert type(od.values()).__hash__ is not None
assert not isinstance(od.keys(), collections.abc.Hashable)
assert not isinstance(od.items(), collections.abc.Hashable)
with assert_raises(TypeError):
    hash(od.keys())
with assert_raises(TypeError):
    hash(od.items())


# A TypeError raised while *comparing* keys (e.g. from a colliding key's
# __eq__) must propagate unchanged, not be rewritten as an unhashable-key
# error. Only genuine hashing failures get the dict-specific "unhashable"
# wording.
class BadEq:
    def __hash__(self):
        return 42  # fixed hash forces a collision

    def __eq__(self, other):
        raise TypeError("nope")


bad = {}
bad[BadEq()] = 1
with assert_raises(TypeError) as cm:
    bad[BadEq()]  # hashes fine (42), then compares against the colliding key
assert "nope" in str(cm.exception), str(cm.exception)
assert "dict key" not in str(cm.exception), (
    f"comparison error mislabeled as unhashable: {cm.exception}"
)

# A genuinely unhashable key still reports the dict-specific message.
with assert_raises(TypeError) as cm:
    {}[[]]
assert "as a dict key" in str(cm.exception), str(cm.exception)

# The message reaches every insertion path, not just __setitem__: the
# constructor, update() and |= all go through the same wrapping.
for make in (
    lambda: dict([([], 1)]),
    lambda: {}.update([([], 1)]),
    lambda: {}.__ior__([([], 1)]),
):
    with assert_raises(TypeError) as cm:
        make()
    assert "as a dict key" in str(cm.exception), str(cm.exception)


# The key is hashed once up front, so a __hash__ that fails only on its first
# call is still reported (a re-hash on the error path would let it escape).
class FlakyHash:
    _calls = 0

    def __hash__(self):
        FlakyHash._calls += 1
        if FlakyHash._calls == 1:
            raise TypeError("first call fails")
        return 0


with assert_raises(TypeError) as cm:
    {}[FlakyHash()]
assert "as a dict key" in str(cm.exception), str(cm.exception)


# The type name is the fully qualified one, like CPython's %T.
def _make_nested():
    class Nested:
        __hash__ = None

    return Nested


with assert_raises(TypeError) as cm:
    {}[_make_nested()()]
assert "_make_nested.<locals>.Nested" in str(cm.exception), str(cm.exception)


# A __hash__ raising a *subclass* of TypeError is left unchanged (CPython
# checks the exact type), so `except MySubclass` still catches it.
class MyTypeError(TypeError):
    pass


class SubclassHash:
    def __hash__(self):
        raise MyTypeError("custom")


with assert_raises(MyTypeError) as cm:
    {}[SubclassHash()]
assert "as a dict key" not in str(cm.exception), str(cm.exception)


# Every operation hashes the key exactly once (the hash is threaded into the
# inner map), so a __hash__ that would fail on a second call is never called
# twice. setdefault() and pop() went through their own lookup before.
class CountingHash:
    calls = 0

    def __hash__(self):
        CountingHash.calls += 1
        if CountingHash.calls >= 2:
            raise TypeError("must not hash twice")
        return 7


CountingHash.calls = 0
{}.setdefault(CountingHash(), 1)
assert CountingHash.calls == 1, CountingHash.calls

CountingHash.calls = 0
with assert_raises(KeyError):
    # A non-empty dict so the lookup must hash the key (CPython skips hashing
    # entirely when popping from an empty dict).
    {1: 1}.pop(CountingHash())
assert CountingHash.calls == 1, CountingHash.calls


# PEP 814 regressions, validated against CPython 3.15.0.
import builtins

if hasattr(builtins, "frozendict"):
    assert (
        frozendict.__doc__
        == """frozendict() -> new empty immutable dictionary
frozendict(mapping) -> new immutable dictionary initialized from a mapping
    object's (key, value) pairs
frozendict(iterable) -> new immutable dictionary initialized as if via:
    d = {}
    for k, v in iterable:
        d[k] = v
    d = frozendict(d)
frozendict(**kwargs) -> new immutable dictionary initialized with the name=value
    pairs in the keyword argument list.  For example:  frozendict(one=1, two=2)"""
    )

    import builtins
    import copy
    import gc
    import pickle
    import types
    import unittest

    class FrozenWithState(frozendict):
        pass

    class FrozenWithSlot(frozendict):
        __slots__ = ("extra",)

    class FrozenDictContractTests(unittest.TestCase):
        def test_construction_identity_and_explicit_new(self):
            original = frozendict(left=object())
            self.assertIs(frozendict(original), original)
            explicit = frozendict.__new__(frozendict, original)
            self.assertIsNot(explicit, original)
            self.assertEqual(explicit, original)
            self.assertIsNot(FrozenWithState(original), original)
            self.assertIsNot(frozendict(FrozenWithState(original)), original)
            original.__init__(object(), object(), ignored=object())
            self.assertEqual(list(original), ["left"])
            self.assertFalse(isinstance(original, dict))
            self.assertFalse(issubclass(frozendict, dict))

        def test_call_ex_identity_preserves_explicit_type_call_distinction(self):
            original = frozendict(answer=42)
            self.assertIs(frozendict(*(original,)), original)
            self.assertIs(frozendict(original, **{}), original)  # noqa: PIE804
            self.assertIs(frozendict(*(original,), **{}), original)  # noqa: PIE804
            self.assertIsNot(type.__call__(frozendict, original), original)
            self.assertIsNot(frozendict.__call__(original), original)

        def test_unrelated_descriptors_cannot_mutate_or_access(self):
            frozen = frozendict(protected=23)
            for method, args in (
                (dict.__init__, ({"replacement": 8},)),
                (dict.__setitem__, ("protected", 99)),
                (dict.__delitem__, ("protected",)),
                (dict.update, ({"replacement": 8},)),
                (dict.clear, ()),
                (dict.get, ("protected",)),
            ):
                with self.subTest(method=method), self.assertRaises(TypeError):
                    method(frozen, *args)
            with self.assertRaises(TypeError):
                frozendict.get({"protected": 23}, "protected")
            self.assertEqual(frozen, {"protected": 23})
            for name in (
                "clear",
                "update",
                "pop",
                "popitem",
                "setdefault",
                "__setitem__",
                "__delitem__",
                "__ior__",
            ):
                self.assertFalse(hasattr(frozen, name), name)

        def test_subclass_missing_is_only_a_subscript_hook(self):
            class Lookup(frozendict):
                def __missing__(self, key):
                    return "missing:" + key

            frozen = Lookup(present=23)
            self.assertEqual(frozen["absent"], "missing:absent")
            self.assertIsNone(frozen.get("absent"))
            self.assertNotIn("absent", frozen)

        def test_fromkeys_constructor_calls_and_source_independence(self):
            calls = []
            seed = frozendict(retained="old", replaced="old")

            class Factory(frozendict):
                def __new__(cls, *args):
                    calls.append(args)
                    if not args:
                        return seed
                    return super().__new__(cls, *args)

                def __setitem__(self, key, value):
                    raise AssertionError("immutable construction called a mutator")

            frozen = Factory.fromkeys(iter(["replaced", "added", "replaced"]), "new")
            self.assertIs(type(frozen), Factory)
            self.assertEqual(
                list(frozen.items()),
                [("retained", "old"), ("replaced", "new"), ("added", "new")],
            )
            self.assertEqual(seed, {"retained": "old", "replaced": "old"})
            self.assertEqual(len(calls), 2)
            self.assertEqual(calls[0], ())
            self.assertIs(type(calls[1][0]), frozendict)
            self.assertEqual(calls[1][0], frozen)

        def test_fromkeys_preserves_custom_constructor_result(self):
            target = {"seed": 1}

            class MutableFactory(frozendict):
                def __new__(cls):
                    return target

            result = MutableFactory.fromkeys(["new"], 2)
            self.assertIs(result, target)
            self.assertEqual(target, {"seed": 1, "new": 2})

        def test_fromkeys_subclass_initialization_is_two_phase(self):
            calls = []

            class Initialized(frozendict):
                def __init__(self, *args):
                    calls.append(args)

            result = Initialized.fromkeys(["first", "second"], 3)
            self.assertIs(type(result), Initialized)
            self.assertEqual(len(calls), 2)
            self.assertEqual(calls[0], ())
            self.assertIs(type(calls[1][0]), frozendict)
            self.assertEqual(calls[1][0], result)

        def test_fromkeys_constructs_before_rejecting_invalid_iterable(self):
            calls = []

            class Factory(frozendict):
                def __new__(cls, *args):
                    calls.append(args)
                    return super().__new__(cls, *args)

            with self.assertRaises(TypeError):
                Factory.fromkeys(123)
            self.assertEqual(calls, [()])

        def test_hash_uses_stored_key_hash_and_caches_values(self):
            class Counted:
                def __init__(self, value):
                    self.value = value
                    self.calls = 0

                def __hash__(self):
                    self.calls += 1
                    return self.value

            key, value = Counted(41), Counted(73)
            frozen = frozendict([(key, value)])
            key_calls = key.calls
            first = hash(frozen)
            self.assertEqual(key.calls, key_calls)
            self.assertEqual(value.calls, 1)
            self.assertEqual(hash(frozen), first)
            self.assertEqual(key.calls, key_calls)
            self.assertEqual(value.calls, 1)

        def test_hash_failure_does_not_poison_cache(self):
            class EventuallyHashable:
                def __init__(self):
                    self.calls = 0

                def __hash__(self):
                    self.calls += 1
                    if self.calls == 1:
                        raise ValueError("not yet")
                    return 97

            value = EventuallyHashable()
            frozen = frozendict(retry=value)
            with self.assertRaisesRegex(ValueError, "not yet"):
                hash(frozen)
            expected = hash(frozen)
            self.assertEqual(hash(frozen), expected)
            self.assertEqual(value.calls, 2)

        def test_mapping_equality_reuses_hashes_but_views_rehash(self):
            class Key:
                forbidden = False

                def __hash__(self):
                    if self.forbidden:
                        raise ValueError("rehash forbidden")
                    return 313

            key = Key()
            left = frozendict([(key, 7)])
            right = frozendict.__new__(frozendict, left)
            mutable = dict(left)
            key.forbidden = True
            self.assertTrue(left == left)  # noqa: PLR0124
            self.assertTrue(left.__eq__(left))
            self.assertTrue(left == right)
            self.assertTrue(left == mutable)
            self.assertTrue(mutable == left)
            for method in ("keys", "items"):
                with self.assertRaisesRegex(ValueError, "rehash forbidden"):
                    _ = getattr(left, method)() == getattr(right, method)()

        def test_value_hash_can_reenter_same_frozendict(self):
            class Reentrant:
                def __init__(self):
                    self.calls = 0
                    self.owner = None
                    self.nested = None

                def __hash__(self):
                    self.calls += 1
                    if self.calls == 1:
                        self.nested = hash(self.owner)
                    return 127

            value = Reentrant()
            frozen = frozendict(recursive=value)
            value.owner = frozen
            result = hash(frozen)
            self.assertEqual(result, value.nested)
            self.assertEqual(hash(frozen), result)
            self.assertEqual(value.calls, 2)

        def test_union_result_type_order_and_identity(self):
            frozen = frozendict(first=1, second=2)
            result = frozen | {"first": 7, "last": 3}
            self.assertIs(type(result), frozendict)
            self.assertEqual(
                list(result.items()), [("first", 7), ("second", 2), ("last", 3)]
            )
            self.assertIs(type({"start": 4} | frozen), dict)
            self.assertIs(frozen | {}, frozen)
            self.assertIs(frozendict() | frozen, frozen)
            child = FrozenWithState(frozen)
            self.assertIs(type(child | {}), frozendict)
            self.assertIsNot(child | {}, child)
            before = frozen
            frozen |= {"third": 3}
            self.assertIsNot(frozen, before)
            self.assertNotIn("third", before)
            with self.assertRaises(TypeError):
                frozen |= [("fourth", 4)]

        def test_explicit_reflected_union_uses_left_operand_type_and_order(self):
            frozen = frozendict(left=1, shared="frozen")
            mutable = {"right": 2, "shared": "mutable"}
            result = dict.__ror__(mutable, frozen)
            self.assertIs(type(result), frozendict)
            self.assertEqual(
                list(result.items()), [("left", 1), ("shared", "mutable"), ("right", 2)]
            )
            result = frozendict.__ror__(frozen, mutable)
            self.assertIs(type(result), dict)
            self.assertEqual(
                list(result.items()), [("right", 2), ("shared", "frozen"), ("left", 1)]
            )

        def test_frozen_globals_and_separate_writable_locals(self):
            observed = []
            frozen = frozendict(answer=42, observed=observed, __builtins__=builtins)
            local = {}
            exec("observed.append(answer); local_answer = answer + 1", frozen, local)
            self.assertEqual(observed, [42])
            self.assertEqual(local, {"local_answer": 43})
            self.assertEqual(eval("answer + 2", frozen), 44)
            self.assertIs(eval("globals()", frozen), frozen)
            self.assertNotIn("local_answer", frozen)

        def test_frozen_globals_require_existing_builtins(self):
            for execute, source in ((eval, "40 + 2"), (exec, "pass")):
                for frozen in (frozendict(answer=42), FrozenWithState(answer=42)):
                    with self.subTest(execute=execute, frozen_type=type(frozen)):
                        with self.assertRaisesRegex(
                            TypeError,
                            "cannot assign __builtins__ to frozendict globals",
                        ):
                            execute(source, frozen, {})
                        self.assertNotIn("__builtins__", frozen)

        def test_functions_from_exec_keep_frozen_globals(self):
            frozen = frozendict(answer=42, __builtins__=builtins)
            local = {}
            exec(
                "def read():\n return answer\ndef assign():\n global answer\n answer = 99\ndef delete():\n global answer\n del answer",
                frozen,
                local,
            )
            self.assertIs(local["read"].__globals__, frozen)
            self.assertEqual(local["read"](), 42)
            for name, suffix in (("assign", "assignment"), ("delete", "deletion")):
                with self.assertRaises(TypeError) as caught:
                    local[name]()
                self.assertEqual(
                    str(caught.exception),
                    "frozendict object does not support item " + suffix,
                )
            with self.assertRaises(TypeError):
                types.FunctionType(local["read"].__code__, frozen)
            self.assertEqual(frozen["answer"], 42)

        def test_frozen_builtins_and_type_namespace(self):
            frozen_builtins = frozendict(len=len)
            frozen = frozendict(__builtins__=frozen_builtins)
            self.assertEqual(eval("len((10, 20))", frozen), 2)
            namespace = frozendict(value=17)
            cls = type("FrozenNamespace", (), namespace)
            self.assertEqual(cls.value, 17)
            cls.value = 19
            self.assertEqual(namespace["value"], 17)

        def test_frozen_subclass_globals_keep_lookup_semantics(self):
            class Namespace(frozendict):
                def __getitem__(self, key):
                    if key == "answer":
                        return 81
                    return super().__getitem__(key)

            frozen = Namespace(answer=42, __builtins__=builtins)
            self.assertEqual(eval("answer", frozen), 81)
            self.assertEqual(eval("(lambda: answer)()", frozen), 81)
            self.assertEqual(eval("answer", frozen, {}), 42)

        def test_class_and_mapping_patterns(self):
            frozen = frozendict(answer=42)
            match frozen:
                case frozendict(captured):
                    self.assertIs(captured, frozen)
                case _:
                    self.fail("frozendict class pattern did not match")
            match frozen:
                case {"answer": captured, **remaining}:
                    self.assertEqual(captured, 42)
                    self.assertEqual(remaining, {})
                    self.assertIs(type(remaining), dict)
                case _:
                    self.fail("frozendict mapping pattern did not match")

        def test_hash_error_wrapper_preserves_exception_subclasses(self):
            class SpecialTypeError(TypeError):
                pass

            class BadKey:
                error = TypeError

                def __hash__(self):
                    raise self.error("intentional hash failure")

            key = BadKey()
            frozen = frozendict(present=1)
            operations = (
                lambda: frozen[key],
                lambda: frozen.get(key),
                lambda: key in frozen,
                lambda: frozendict([(key, 1)]),
                lambda: frozendict.fromkeys([key]),
            )
            for error in (TypeError, SpecialTypeError, ValueError):
                key.error = error
                for operation in operations:
                    with self.subTest(error=error, operation=operation):
                        with self.assertRaises(error) as caught:
                            operation()
                        self.assertIs(type(caught.exception), error)
                        expected = "intentional hash failure"
                        if error is TypeError:
                            name = BadKey.__qualname__
                            if BadKey.__module__ not in ("builtins", "__main__"):
                                name = BadKey.__module__ + "." + name
                            expected = f"cannot use '{name}' as a frozendict key (intentional hash failure)"
                        self.assertEqual(str(caught.exception), expected)

        def test_copy_protocol_and_getnewargs_do_not_expose_storage(self):
            frozen = frozendict(present=object())
            self.assertIs(copy.copy(frozen), frozen)
            self.assertIs(frozen.copy(), frozen)
            args = frozen.__getnewargs__()
            self.assertIs(type(args), tuple)
            self.assertIs(type(args[0]), dict)
            args[0].clear()
            self.assertEqual(list(frozen), ["present"])
            child = FrozenWithState(frozen)
            child.extra = "state"
            shallow = copy.copy(child)
            self.assertIs(type(shallow), FrozenWithState)
            self.assertEqual(shallow.extra, "state")
            self.assertIs(type(child.copy()), frozendict)
            deep = copy.deepcopy(frozen)
            self.assertIsNot(deep, frozen)

        def test_subclass_copy_dispatch_tracks_iterator_override(self):
            class NativeIteration(frozendict):
                def keys(self):
                    return ["virtual"]

                def __getitem__(self, key):
                    return 88

            class CustomIteration(NativeIteration):
                def __iter__(self):
                    return iter(["virtual"])

            native = NativeIteration(stored=7)
            custom = CustomIteration(stored=7)
            self.assertEqual(native.copy(), {"stored": 7})
            self.assertEqual(native | {"tail": 9}, {"stored": 7, "tail": 9})
            self.assertEqual(NativeIteration.fromkeys(["tail"], 9), {"tail": 9})
            self.assertEqual(custom.copy(), {"virtual": 88})
            self.assertEqual(custom | {"tail": 9}, {"virtual": 88, "tail": 9})
            self.assertEqual(
                CustomIteration.fromkeys(["tail"], 9), {"virtual": 88, "tail": 9}
            )

        def test_deepcopy_recursive_aliases_from_both_roots(self):
            loop = []
            frozen = frozendict(left=loop, right=loop)
            loop.extend([frozen, frozen])
            for root in (frozen, loop):
                result = copy.deepcopy(root)
                copied_frozen = result if isinstance(result, frozendict) else result[0]
                copied_loop = copied_frozen["left"]
                self.assertIsNot(copied_frozen, frozen)
                self.assertIsNot(copied_loop, loop)
                self.assertIs(copied_frozen["right"], copied_loop)
                self.assertIs(copied_loop[0], copied_frozen)
                self.assertIs(copied_loop[1], copied_frozen)

        def test_pickle_recursive_payload_and_subclass_state(self):
            for cls in (frozendict, FrozenWithState, FrozenWithSlot):
                loop = []
                frozen = cls(left=loop, right=loop)
                loop.extend([frozen, frozen])
                if cls is not frozendict:
                    frozen.extra = loop
                for protocol in range(pickle.HIGHEST_PROTOCOL + 1):
                    with self.subTest(cls=cls, protocol=protocol):
                        if protocol < 2:
                            with self.assertRaises(TypeError):
                                pickle.dumps(frozen, protocol)
                            continue
                        result = pickle.loads(pickle.dumps(frozen, protocol))
                        self.assertIs(type(result), cls)
                        self.assertIsNot(result, frozen)
                        self.assertIs(result["left"], result["right"])
                        self.assertIs(result["left"][0], result)
                        self.assertIs(result["left"][1], result)
                        if cls is not frozendict:
                            self.assertIs(result.extra, result["left"])

        def test_gc_does_not_expose_mutable_backing_mapping(self):
            marker = object()
            frozen = frozendict(private_payload=marker)
            for referent in gc.get_referents(frozen):
                self.assertFalse(isinstance(referent, dict))
            self.assertIs(frozen["private_payload"], marker)

        def test_gc_does_not_publish_partly_constructed_mapping(self):
            marker = object()
            observed = []

            def source():
                yield "private_payload", marker
                observed.extend(
                    candidate
                    for candidate in gc.get_objects()
                    if type(candidate) is frozendict
                    and candidate.get("private_payload") is marker
                )
                yield "finished", True

            frozen = frozendict(source())
            self.assertEqual(observed, [])
            self.assertIs(frozen["private_payload"], marker)
            self.assertIs(frozen["finished"], True)

    import gc
    import operator
    import unittest
    import weakref

    class FrozenDictEdgeCaseTests(unittest.TestCase):
        def test_hash_matches_frozenset_of_items(self):
            cases = (
                frozendict(),
                frozendict([(1, 2), (-3, 4), (2**130, -7)]),
                frozendict(first=1, second=2),
                frozendict(second=2, first=1),
                frozendict([(1, "one"), ("two", 2), (("three",), frozenset({3}))]),
            )
            for frozen in cases:
                with self.subTest(frozen=frozen):
                    self.assertEqual(hash(frozen), hash(frozenset(frozen.items())))

        def test_gc_cycles_and_class_reassignment_cannot_expose_mutation(self):
            frozen = frozendict(protected=23)
            original_hash = hash(frozen)
            with self.assertRaises(TypeError):
                frozen.__class__ = dict
            self.assertIs(type(frozen), frozendict)
            self.assertEqual(frozen, {"protected": 23})
            self.assertEqual(hash(frozen), original_hash)

            class Marker:
                pass

            def make_cycle(include_views):
                marker = Marker()
                loop = [marker]
                owner = frozendict(loop=loop)
                loop.append(owner)
                if include_views:
                    loop.extend(
                        (owner.keys(), owner.values(), owner.items(), iter(owner))
                    )
                return weakref.ref(marker)

            for include_views in (False, True):
                with self.subTest(include_views=include_views):
                    marker_ref = make_cycle(include_views)
                    gc.collect()
                    self.assertIsNone(marker_ref())

        def test_union_subclass_dispatch_matrix(self):
            for base in (dict, frozendict):

                class NativeIteration(base):
                    def keys(self):
                        return ["virtual"]

                    def __getitem__(self, key):
                        return 88

                class CustomIteration(NativeIteration):
                    def __iter__(self):
                        return iter(["virtual"])

                for cls in (NativeIteration, CustomIteration):
                    custom = cls is CustomIteration
                    for storage in ({}, {"stored": 7}):
                        subject = cls(storage)
                        copied = {"virtual": 88} if custom and storage else storage
                        merged = {"virtual": 88} if custom else storage
                        cases = (
                            ("left_dict", lambda: subject | {}, base, copied),
                            (
                                "left_frozen",
                                lambda: subject | frozendict(),
                                base,
                                copied,
                            ),
                            ("right_dict", lambda: {} | subject, dict, merged),
                            (
                                "right_frozen",
                                lambda: frozendict() | subject,
                                frozendict,
                                merged,
                            ),
                            (
                                "construction",
                                lambda: frozendict(subject),
                                frozendict,
                                merged,
                            ),
                        )
                        for name, operation, expected_type, expected in cases:
                            with self.subTest(
                                base=base, cls=cls, storage=storage, operation=name
                            ):
                                result = operation()
                                self.assertIs(type(result), expected_type)
                                self.assertEqual(result, expected)
                                self.assertEqual(
                                    list(subject.items()), list(storage.items())
                                )

                        if base is frozendict:
                            with self.subTest(
                                base=base, cls=cls, storage=storage, operation="copy"
                            ):
                                result = subject.copy()
                                self.assertIs(type(result), frozendict)
                                self.assertIsNot(result, subject)
                                self.assertEqual(result, copied)

        def test_mapping_equality_uses_left_value_and_stored_key_hash(self):
            trace = []

            class Key:
                def __init__(self, side):
                    self.side = side

                def __hash__(self):
                    trace.append(("hash", self.side))
                    return 313

                def __eq__(self, other):
                    trace.append(("key", self.side, other.side))
                    return True

            class Value:
                def __init__(self, side):
                    self.side = side

                def __eq__(self, other):
                    trace.append(("value", self.side, other.side))
                    return self.side == "left"

            for left_type in (dict, frozendict):
                for right_type in (dict, frozendict):
                    left = left_type([(Key("left"), Value("left"))])
                    right = right_type([(Key("right"), Value("right"))])
                    with self.subTest(left_type=left_type, right_type=right_type):
                        trace.clear()
                        self.assertTrue(left == right)
                        self.assertEqual(
                            trace,
                            [("key", "right", "left"), ("value", "left", "right")],
                        )
                        trace.clear()
                        self.assertFalse(right == left)
                        self.assertEqual(
                            trace,
                            [("key", "left", "right"), ("value", "right", "left")],
                        )

        def test_view_comparison_direction_and_asymmetric_values(self):
            trace = []

            class Key:
                def __init__(self, side):
                    self.side = side

                def __hash__(self):
                    trace.append(("hash", self.side))
                    return 313

                def __eq__(self, other):
                    trace.append(("key", self.side, other.side))
                    return True

            class Value:
                def __init__(self, side):
                    self.side = side

                def __eq__(self, other):
                    trace.append(("value", self.side, other.side))
                    return self.side == "left"

            for left_type in (dict, frozendict):
                for right_type in (dict, frozendict):
                    left = left_type([(Key("left"), Value("left"))])
                    right = right_type([(Key("right"), Value("right"))])
                    for method in ("keys", "items"):
                        for op in (
                            operator.eq,
                            operator.ne,
                            operator.lt,
                            operator.le,
                            operator.gt,
                            operator.ge,
                        ):
                            with self.subTest(
                                left_type=left_type,
                                right_type=right_type,
                                method=method,
                                operation=op.__name__,
                            ):
                                trace.clear()
                                result = op(
                                    getattr(left, method)(), getattr(right, method)()
                                )
                                if op in (operator.lt, operator.gt):
                                    self.assertFalse(result)
                                    self.assertEqual(trace, [])
                                    continue
                                reverse = op is operator.ge
                                needle, container = (
                                    ("right", "left") if reverse else ("left", "right")
                                )
                                expected_trace = [
                                    ("hash", needle),
                                    ("key", container, needle),
                                ]
                                contained = True
                                if method == "items":
                                    expected_trace.append(("value", container, needle))
                                    contained = container == "left"
                                self.assertEqual(
                                    result,
                                    not contained if op is operator.ne else contained,
                                )
                                self.assertEqual(trace, expected_trace)

        def test_view_hash_error_context_comes_from_containment_mapping(self):
            class Key:
                broken = False

                def __hash__(self):
                    if self.broken:
                        raise TypeError("intentional view hash error")
                    return 313

            for left_type in (dict, frozendict):
                for right_type in (dict, frozendict):
                    key = Key()
                    left = left_type([(key, 1)])
                    right = right_type([(key, 1)])
                    key.broken = True
                    for method in ("keys", "items"):
                        for op in (operator.eq, operator.ne, operator.le, operator.ge):
                            with self.subTest(
                                left_type=left_type,
                                right_type=right_type,
                                method=method,
                                operation=op.__name__,
                            ):
                                container = (
                                    left_type if op is operator.ge else right_type
                                )
                                with self.assertRaises(TypeError) as caught:
                                    op(
                                        getattr(left, method)(),
                                        getattr(right, method)(),
                                    )
                                self.assertTrue(
                                    str(caught.exception).endswith(
                                        f"as a {container.__name__} key (intentional view hash error)"
                                    ),
                                    str(caught.exception),
                                )

        def test_view_equality_checks_mutation_of_the_iterated_mapping(self):
            for op in (operator.eq, operator.le, operator.ge):
                left = {"key": object()}

                class Value:
                    def __eq__(self, other):
                        left.clear()
                        return True

                right = frozendict(key=Value())
                with self.subTest(operation=op.__name__):
                    if op is operator.ge:
                        self.assertTrue(op(left.items(), right.items()))
                    else:
                        with self.assertRaisesRegex(
                            RuntimeError, "dictionary changed size during iteration"
                        ):
                            op(left.items(), right.items())
                    self.assertEqual(left, {})

        def test_fromkeys_merges_constructor_result_before_iterable_validation(self):
            calls = []

            class Source(frozendict):
                def __iter__(self):
                    return iter(())

                def keys(self):
                    calls.append("keys")
                    return []

            source = Source()

            class Factory(frozendict):
                def __new__(cls, *args):
                    calls.append("new")
                    return source

            with self.assertRaises(TypeError):
                Factory.fromkeys(123)
            self.assertEqual(calls, ["new", "keys"])
            self.assertEqual(source, {})

        def test_fromkeys_source_error_precedes_invalid_iterable(self):
            class Source(frozendict):
                def __iter__(self):
                    return iter(())

                def keys(self):
                    raise ValueError("source merge failed")

            source = Source()

            class Factory(frozendict):
                def __new__(cls, *args):
                    return source

            with self.assertRaisesRegex(ValueError, "source merge failed"):
                Factory.fromkeys(123)

    import builtins
    import gc
    import sys
    import types
    import unittest

    class FrozenGlobalsTests(unittest.TestCase):
        def setUp(self):
            self.ns = frozendict(
                __builtins__=builtins.__dict__,
                __name__="frozen_test",
                answer=42,
                sys=sys,
            )

        def test_eval_and_implicit_scope(self):
            self.assertIs(eval("globals()", self.ns), self.ns)
            self.assertIs(eval("locals()", self.ns), self.ns)
            self.assertEqual(eval("eval('answer')", self.ns), 42)
            self.assertEqual(eval("(lambda: answer)()", self.ns), 42)

        def test_function_and_materialized_frame(self):
            local = {}
            exec(
                "def f():\n return sys._getframe(), globals(), answer\n", self.ns, local
            )
            f = local["f"]
            self.assertIs(f.__globals__, self.ns)
            self.assertEqual(f.__module__, "frozen_test")
            for _ in range(300):
                frame, ns, answer = f()
                self.assertIs(frame.f_globals, self.ns)
                self.assertIs(ns, self.ns)
                self.assertEqual(answer, 42)
            gc.collect()
            self.assertIs(frame.f_globals, self.ns)

        def test_generator_frames(self):
            local = {}
            exec(
                "def gen():\n yield sys._getframe(), globals(), answer\n yield answer\n",
                self.ns,
                local,
            )
            gen = local["gen"]()
            self.assertIs(gen.gi_frame.f_globals, self.ns)
            frame, ns, answer = next(gen)
            self.assertIs(frame.f_globals, self.ns)
            self.assertIs(ns, self.ns)
            self.assertEqual(answer, 42)
            self.assertEqual(next(gen), 42)
            with self.assertRaises(StopIteration):
                next(gen)
            gc.collect()
            self.assertIs(frame.f_globals, self.ns)

        def test_coroutine_frames(self):
            local = {}
            exec(
                "async def coro():\n return sys._getframe(), globals(), answer\n",
                self.ns,
                local,
            )
            coro = local["coro"]()
            self.assertIs(coro.cr_frame.f_globals, self.ns)
            with self.assertRaises(StopIteration) as stopped:
                coro.send(None)
            frame, ns, answer = stopped.exception.value
            self.assertIs(frame.f_globals, self.ns)
            self.assertIs(ns, self.ns)
            self.assertEqual(answer, 42)

        def test_async_generator_frames(self):
            local = {}
            exec(
                "async def agen():\n yield sys._getframe(), globals(), answer\n",
                self.ns,
                local,
            )
            agen = local["agen"]()
            self.assertIs(agen.ag_frame.f_globals, self.ns)
            with self.assertRaises(StopIteration) as stopped:
                agen.asend(None).send(None)
            frame, ns, answer = stopped.exception.value
            self.assertIs(frame.f_globals, self.ns)
            self.assertIs(ns, self.ns)
            self.assertEqual(answer, 42)
            with self.assertRaises(StopIteration):
                agen.aclose().send(None)

        def test_local_namespace_and_imports(self):
            local = {}
            exec(
                "import math\nclass C:\n value = answer\ndef f():\n import math\n return math.sqrt(answer)\n",
                self.ns,
                local,
            )
            self.assertEqual(local["C"].value, 42)
            self.assertEqual(local["C"].__module__, "frozen_test")
            self.assertEqual(local["f"](), local["math"].sqrt(42))
            self.assertNotIn("math", self.ns)

        def test_missing_builtins(self):
            for func, source in [(eval, ""), (exec, "")]:
                with self.assertRaises(TypeError) as caught:
                    func(source, frozendict())
                self.assertEqual(
                    str(caught.exception),
                    "cannot assign __builtins__ to frozendict globals",
                )

        def test_writes_and_deletes(self):
            for source, error in [
                ("answer = 0", "'frozendict' object does not support item assignment"),
                (
                    "global answer; answer = 0",
                    "frozendict object does not support item assignment",
                ),
                (
                    "global answer; del answer",
                    "frozendict object does not support item deletion",
                ),
                (
                    "global missing; del missing",
                    "frozendict object does not support item deletion",
                ),
            ]:
                with self.assertRaises(TypeError) as caught:
                    exec(source, self.ns)
                self.assertEqual(str(caught.exception), error)
            self.assertEqual(self.ns["answer"], 42)
            with self.assertRaises(NameError):
                exec("del answer", self.ns)

        def test_explicit_function_constructor_stays_dict_only(self):
            with self.assertRaises(TypeError):
                types.FunctionType((lambda: 42).__code__, self.ns)

        def test_subclass_name_and_global_lookup(self):
            class F(frozendict):
                def __getitem__(self, key):
                    if key == "answer":
                        return 81
                    return super().__getitem__(key)

            ns = F(self.ns)
            self.assertEqual(eval("answer", ns), 81)
            self.assertEqual(eval("(lambda: answer)()", ns), 81)
            self.assertEqual(eval("answer", ns, {}), 42)

        def test_subclass_function_metadata_reads_storage(self):
            class F(frozendict):
                def __getitem__(self, key):
                    if key == "__name__":
                        return "wrong_name"
                    if key == "__builtins__":
                        return {"len": lambda x: 99}
                    return super().__getitem__(key)

            ns = F(self.ns)
            f = eval("lambda: len([])", ns)
            self.assertEqual(f.__module__, "frozen_test")
            self.assertIs(f.__globals__, ns)
            self.assertEqual(f(), 0)

        def test_subclass_builtins_assignment_hook(self):
            calls = []

            class F(frozendict):
                def __setitem__(self, key, value):
                    calls.append((key, value))

            for operation, source, mode, expected in (
                (eval, "1", "eval", 1),
                (exec, "pass", "exec", None),
            ):
                namespace = F()
                calls.clear()
                self.assertEqual(
                    operation(compile(source, "<frozen-globals>", mode), namespace),
                    expected,
                )
                self.assertEqual(calls, [("__builtins__", builtins.__dict__)])
                self.assertNotIn("__builtins__", namespace)
                calls.clear()
                with self.assertRaisesRegex(
                    TypeError, "frozendict object does not support item assignment"
                ):
                    operation(source, namespace)
                self.assertEqual(calls, [("__builtins__", builtins.__dict__)])

        def test_implicit_frozen_builtins_namespace_counts(self):
            import _testinternalcapi

            function = eval(
                "lambda: len(())",
                frozendict(__builtins__=frozendict(len=len)),
            )
            counts = _testinternalcapi.get_code_var_counts(function)
            self.assertEqual(
                counts["unbound"]["globals"],
                {"total": 1, "numglobal": 0, "numbuiltin": 1, "numunknown": 0},
            )
            for argument in ("globalsns", "builtinsns"):
                with self.assertRaises(TypeError):
                    _testinternalcapi.get_code_var_counts(
                        function, **{argument: frozendict()}
                    )

        def test_warning_context_reads_frozen_storage(self):
            import warnings

            class F(frozendict):
                def __getitem__(self, key):
                    if key in ("__warningregistry__", "__name__"):
                        raise AssertionError("warning context must read stored entries")
                    return super().__getitem__(key)

            for has_registry in (False, True):
                entries = dict(self.ns, warn=warnings.warn)
                if has_registry:
                    entries["__warningregistry__"] = {}
                function = eval("lambda: warn('probe')", F(entries))
                with warnings.catch_warnings(record=True) as caught:
                    warnings.simplefilter("always")
                    if has_registry:
                        function()
                        self.assertEqual(len(caught), 1)
                        self.assertEqual(str(caught[0].message), "probe")
                    else:
                        with self.assertRaisesRegex(
                            TypeError,
                            "frozendict object does not support item assignment",
                        ):
                            function()
                        self.assertEqual(caught, [])

        def test_frozen_builtins(self):
            ns = frozendict(__builtins__=frozendict({"len": len}))
            self.assertEqual(eval("(lambda: len([1, 2]))()", ns), 2)

        def test_shared_code_does_not_keep_mutable_global_specialization(self):
            code = compile("lambda: answer", "<globals-probe>", "eval")
            mutable = {"__builtins__": builtins.__dict__, "answer": 19}
            f = eval(code, mutable)
            for _ in range(300):
                self.assertEqual(f(), 19)
            frozen_f = eval(code, self.ns)
            for _ in range(300):
                self.assertEqual(frozen_f(), 42)
                self.assertEqual(f(), 19)
            mutable["answer"] = 23
            self.assertEqual(f(), 23)
            self.assertEqual(frozen_f(), 42)

    import types
    import unittest

    class FrozenTypeNamespaceTests(unittest.TestCase):
        def test_exact_frozen_namespace_is_copied(self):
            namespace = frozendict(value=17, __qualname__="Outer.FrozenNamespace")
            original_hash = hash(namespace)
            cls = type("FrozenNamespace", (), namespace)
            self.assertEqual(cls.value, 17)
            self.assertEqual(cls.__qualname__, "Outer.FrozenNamespace")
            cls.value = 19
            cls.added = 23
            del cls.value
            self.assertEqual(
                namespace,
                {"value": 17, "__qualname__": "Outer.FrozenNamespace"},
            )
            self.assertEqual(hash(namespace), original_hash)
            self.assertNotIn("__module__", namespace)
            self.assertNotIn("added", namespace)

        def test_builtin_iterator_bypasses_subclass_mapping_overrides(self):
            for base in (dict, frozendict):

                class Namespace(base):
                    def keys(self):
                        raise AssertionError("keys must not be called")

                    def __getitem__(self, key):
                        raise AssertionError("__getitem__ must not be called")

                    def __len__(self):
                        raise AssertionError("__len__ must not be called")

                with self.subTest(base=base):
                    namespace = Namespace(value=17, __qualname__="Outer.C")
                    cls = type("C", (), namespace)
                    self.assertEqual(cls.value, 17)
                    self.assertEqual(cls.__qualname__, "Outer.C")
                    self.assertEqual(base.__getitem__(namespace, "value"), 17)

        def test_custom_iterator_uses_mapping_order_and_values(self):
            for base in (dict, frozendict):
                calls = []

                class Namespace(base):
                    def __iter__(self):
                        raise AssertionError("the copy must use keys, not __iter__")

                    def keys(self):
                        calls.append("keys")
                        return ["second", "first"]

                    def __getitem__(self, key):
                        calls.append(key)
                        return {"first": 11, "second": 22}[key]

                with self.subTest(base=base):
                    namespace = Namespace(stored=3)
                    cls = type("C", (), namespace)
                    self.assertEqual(calls, ["keys", "second", "first"])
                    self.assertEqual((cls.first, cls.second), (11, 22))
                    self.assertFalse(hasattr(cls, "stored"))
                    self.assertEqual(
                        [key for key in cls.__dict__ if not key.startswith("__")],
                        ["second", "first"],
                    )
                    self.assertEqual(base.__getitem__(namespace, "stored"), 3)

        def test_empty_subclass_copy_bypasses_mapping_overrides(self):
            for base in (dict, frozendict):

                class Namespace(base):
                    def __iter__(self):
                        raise AssertionError("__iter__ must not be called")

                    def keys(self):
                        raise AssertionError("keys must not be called")

                    def __getitem__(self, key):
                        raise AssertionError("__getitem__ must not be called")

                    def __len__(self):
                        return 1

                with self.subTest(base=base):
                    namespace = Namespace()
                    cls = type("C", (), namespace)
                    self.assertEqual(cls.__name__, "C")
                    self.assertEqual(base.__len__(namespace), 0)

        def test_class_cells_use_a_mutable_copy(self):
            classcell = types.CellType()
            dictcell = types.CellType()
            namespace = frozendict(
                value=17,
                __classcell__=classcell,
                __classdictcell__=dictcell,
            )
            cls = type("C", (), namespace)
            self.assertIs(classcell.cell_contents, cls)
            classdict = dictcell.cell_contents
            self.assertIs(type(classdict), dict)
            self.assertIsNot(classdict, namespace)
            self.assertNotIn("__classcell__", classdict)
            self.assertNotIn("__classdictcell__", classdict)
            self.assertIs(namespace["__classcell__"], classcell)
            self.assertIs(namespace["__classdictcell__"], dictcell)
            cls.value = 19
            self.assertEqual(classdict["value"], 19)
            self.assertEqual(namespace["value"], 17)

        def test_descriptor_initialization_does_not_change_source(self):
            calls = []

            class Descriptor:
                def __set_name__(self, owner, name):
                    calls.append((owner, name))
                    owner.added = 23

            descriptor = Descriptor()
            namespace = frozendict(field=descriptor)
            cls = type("C", (), namespace)
            self.assertEqual(calls, [(cls, "field")])
            self.assertEqual(cls.added, 23)
            self.assertIs(namespace["field"], descriptor)
            self.assertEqual(list(namespace), ["field"])

        def test_winning_metaclass_receives_original_namespace(self):
            for base in (dict, frozendict):

                class Namespace(base):
                    def __iter__(self):
                        raise AssertionError("namespace must not be copied")

                    def keys(self):
                        raise AssertionError("namespace must not be copied")

                namespace = Namespace(value=17)

                class Meta(type):
                    def __new__(metacls, name, bases, ns):
                        if name == "Child":
                            return ns
                        return super().__new__(metacls, name, bases, ns)

                class Base(metaclass=Meta):
                    pass

                with self.subTest(base=base):
                    self.assertIs(type("Child", (Base,), namespace), namespace)

        def test_keywords_and_explicit_type_new(self):
            calls = []

            class Base:
                def __init_subclass__(cls, *, marker):
                    calls.append((cls, marker))

            namespace = frozendict(value=17)
            cls = type("C", (Base,), namespace, marker=23)
            explicit = type.__new__(type, "D", (Base,), namespace, marker=29)
            self.assertEqual(calls, [(cls, 23), (explicit, 29)])
            self.assertEqual((cls.value, explicit.value), (17, 17))
            self.assertEqual(namespace, {"value": 17})

        def test_non_dictionary_mappings_remain_rejected(self):
            class Mapping:
                def keys(self):
                    return ["value"]

                def __getitem__(self, key):
                    return 17

            for namespace in (types.MappingProxyType({}), Mapping(), [], ()):
                with self.subTest(namespace=namespace), self.assertRaises(TypeError):
                    type("C", (), namespace)

    frozen_suite = unittest.TestSuite(
        unittest.defaultTestLoader.loadTestsFromTestCase(case)
        for case in (
            FrozenDictContractTests,
            FrozenDictEdgeCaseTests,
            FrozenGlobalsTests,
            FrozenTypeNamespaceTests,
        )
    )
    assert frozen_suite.countTestCases() == 61
    frozen_result = unittest.TextTestRunner(verbosity=2).run(frozen_suite)
    assert frozen_result.wasSuccessful()
    assert not frozen_result.skipped
