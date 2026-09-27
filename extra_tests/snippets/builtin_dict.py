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
