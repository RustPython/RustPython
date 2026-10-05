"""Private exact-dict relocation and callback-reentry regression coverage."""

import _imp
import sys


def raises(kind, call, expected=None):
    try:
        call()
    except kind as exc:
        if expected is not None:
            assert exc is expected
        return exc
    raise AssertionError(f"{kind.__name__} was not raised")


def check_native_helper():
    move = _imp._dict_move_to_end
    first, middle, last = object(), object(), object()
    mapping = {"first": first, "middle": middle, "last": last}
    assert move(mapping, "middle") is middle
    assert list(mapping) == ["first", "last", "middle"]
    assert len(mapping) == 3
    assert move(mapping, "middle") is middle
    assert list(mapping) == ["first", "last", "middle"]
    single = {"only": None}
    assert move(single, "only") is None
    assert single == {"only": None}
    missing = object()
    exc = raises(KeyError, lambda: move(mapping, missing))
    assert exc.args[0] is missing
    assert list(mapping) == ["first", "last", "middle"]

    class Subclass(dict):
        pass

    raises(TypeError, lambda: move(Subclass(mapping), "middle"))
    raises(TypeError, lambda: move([], "middle"))
    stored = "".join(["equal", " key"])
    equal = "".join(["equal ", "key"])
    assert stored == equal and stored is not equal
    mapping = {stored: middle, "after": last}
    assert move(mapping, equal) is middle
    assert list(mapping)[-1] is stored

    failure = ValueError("hash failure")

    class Hashed(str):
        count = 0
        fail = False

        def __hash__(self):
            self.count += 1
            if self.fail:
                raise failure
            return super().__hash__()

    lookup = Hashed(stored)
    assert move(mapping, lookup) is middle
    assert lookup.count == 1
    lookup.fail = True
    raises(ValueError, lambda: move(mapping, lookup), failure)
    assert lookup.count == 2

    class Collision:
        def __init__(self, label, action=None):
            self.label = label
            self.action = action

        def __hash__(self):
            return 42

        def __eq__(self, other):
            action, self.action = self.action, None
            if action is not None:
                action()
            return isinstance(other, Collision) and self.label == other.label

    lookup = Collision("target")
    stored = Collision("target")
    impostor = Collision("different")
    mapping = {stored: first, "after": last}

    def replace_key():
        mapping.clear()
        mapping[impostor] = middle

    stored.action = replace_key
    raises(KeyError, lambda: move(mapping, lookup))
    assert list(mapping) == [impostor]
    assert mapping[impostor] is middle

    stored = Collision("target")
    mapping = {stored: first, "after": last}
    stored.action = lambda: mapping.__setitem__(stored, middle)
    assert move(mapping, lookup) is middle
    assert list(mapping)[-1] is stored

    stored = Collision("target")
    mapping = {stored: first, "after": last}
    stored.action = lambda: mapping.clear()
    raises(KeyError, lambda: move(mapping, lookup))
    assert mapping == {}

    stored = Collision("target")
    mapping = {stored: first, "after": last}

    def resize():
        mapping.clear()
        for i in range(100):
            mapping[i] = i
        mapping[stored] = middle
        mapping["after"] = last

    stored.action = resize
    assert move(mapping, lookup) is middle
    assert list(mapping)[-1] is stored
    assert len(mapping) == 102

    # A surviving false candidate alone does not certify the probe prefix.
    # Nested relocation compacts T into P's earlier bucket, while A keeps its
    # key identity, bucket and physical entry index. T never becomes absent.
    prefix_key = Collision("prefix")
    candidate = Collision("candidate")
    target = Collision("target")
    mapping = {prefix_key: first, candidate: last, target: middle}

    def compact_prefix():
        del mapping[prefix_key]
        assert move(mapping, candidate) is last
        assert list(mapping) == [target, candidate]

    candidate.action = compact_prefix
    assert move(mapping, lookup) is middle
    assert list(mapping) == [candidate, target]

    # Inserting in a previously visited DUMMY bucket also requires a restart.
    mapping = {prefix_key: first, candidate: last}
    del mapping[prefix_key]
    candidate.action = lambda: mapping.__setitem__(lookup, middle)
    assert move(mapping, lookup) is middle
    assert list(mapping) == [candidate, lookup]

    failure = RuntimeError("equality failure")

    def fail():
        raise failure

    stored = Collision("target", fail)
    mapping = {stored: first, "after": last}
    before = list(mapping)
    raises(RuntimeError, lambda: move(mapping, lookup), failure)
    assert list(mapping) == before

    class HashMutation:
        def __hash__(self):
            mapping.clear()
            mapping["kept"] = last
            return 42

    raises(KeyError, lambda: move(mapping, HashMutation()))
    assert mapping == {"kept": last}

    # Releasing the removed candidate must not run its finalizer under the
    # dictionary lock. The only owners are the dict and the lookup's witness.
    finalized = []

    class Finalized(Collision):
        def __del__(self):
            mapping["finalized"] = True
            finalized.append(True)

    mapping = {Finalized("target", lambda: mapping.clear()): first}
    raises(KeyError, lambda: move(mapping, lookup))
    assert finalized == [True]
    assert mapping == {"finalized": True}

    mapping = {"a": 1, "b": 2}
    for i in range(100):
        forward, backward = iter(mapping), reversed(mapping)
        key = "a" if i % 2 == 0 else "b"
        move(mapping, key)
        raises(RuntimeError, lambda: next(forward))
        raises(RuntimeError, lambda: next(backward))
        assert list(mapping)[-1] == key


if sys.implementation.name == "rustpython":
    check_native_helper()
