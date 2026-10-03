"""Import-cache ordering must not temporarily remove an exact-dict entry.

The native relocation helper and trace-visible presence assertion protect
RustPython's module-cache atomicity and callback reentry boundaries.
"""

import _imp
import os
import sys
import types
from importlib import _bootstrap as frozen


def raises(kind, call, expected=None):
    try:
        call()
    except kind as exc:
        if expected is not None:
            assert exc is expected
        return exc
    raise AssertionError(f"{kind.__name__} was not raised")


def source_bootstrap():
    path = os.path.join(
        os.path.dirname(os.path.dirname(os.path.dirname(__file__))),
        "Lib",
        "importlib",
        "_bootstrap.py",
    )
    source = types.ModuleType("atomic_reorder_source_bootstrap")
    with open(path, encoding="utf-8") as stream:
        exec(compile(stream.read(), path, "exec"), source.__dict__)
    for name in (
        "sys",
        "_imp",
        "_thread",
        "_warnings",
        "_weakref",
        "_bootstrap_external",
    ):
        setattr(source, name, getattr(frozen, name))
    source._blocking_on = source._WeakValueDictionary()
    return source


class Loader:
    def __init__(self, action):
        self.action = action

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        self.action(module)


def check_trace_presence(bootstrap, mode):
    original = sys.modules
    old_trace = sys.gettrace()
    name = "_atomic_reorder_trace"
    expected = types.ModuleType(name)
    after = "_atomic_reorder_trace_after"
    state = {"armed": False, "events": 0}
    failure = RuntimeError("legacy loader failed with its entry still present")
    codes = {
        bootstrap._exec.__code__,
        bootstrap._load_unlocked.__code__,
    }
    if hasattr(bootstrap, "_load_backward_compatible"):
        codes.add(bootstrap._load_backward_compatible.__code__)
    if hasattr(bootstrap, "_reorder_module"):
        codes.add(bootstrap._reorder_module.__code__)

    def trace(frame, event, arg):
        if event == "line" and state["armed"] and frame.f_code in codes:
            state["events"] += 1
            assert name in sys.modules, (mode, frame.f_code.co_name, frame.f_lineno)
            assert sys.modules[name] is expected
        return trace

    def finish(module):
        sys.modules[name] = expected
        sys.modules[after] = object()
        state["armed"] = True

    class LegacyLoader:
        def load_module(self, fullname):
            assert fullname == name
            finish(expected)
            if mode == "legacy_error":
                raise failure
            return expected

    try:
        sys.modules = original.copy()
        loader = LegacyLoader() if mode.startswith("legacy") else Loader(finish)
        spec = bootstrap.ModuleSpec(name, loader)
        if mode == "exec":
            initial = bootstrap.module_from_spec(spec)
            sys.modules[name] = initial
            call = lambda: bootstrap._exec(spec, initial)
        elif mode.startswith("legacy"):
            call = lambda: bootstrap._load_backward_compatible(spec)
        else:
            call = lambda: bootstrap._load_unlocked(spec)
        sys.settrace(trace)
        if mode == "legacy_error":
            raises(RuntimeError, call, failure)
        else:
            assert call() is expected
        sys.settrace(old_trace)
        assert state["events"] > 0, "the successful loader window was not traced"
        assert list(sys.modules)[-2:] == [after, name]
    finally:
        sys.settrace(old_trace)
        sys.modules = original


def check_custom_mappings(bootstrap):
    original = sys.modules
    name = "_atomic_reorder_custom"
    value = object()
    events = []

    class Subclass(dict):
        def pop(self, key):
            events.append(("pop", key))
            return super().pop(key)

        def __setitem__(self, key, value):
            events.append(("set", key, value))
            super().__setitem__(key, value)

    class Mapping:
        def __init__(self, pairs):
            self.data = dict(pairs)

        def pop(self, key):
            events.append(("pop", key))
            return self.data.pop(key)

        def __setitem__(self, key, value):
            events.append(("set", key, value))
            self.data[key] = value

    try:
        spec = bootstrap.ModuleSpec(name, None)
        for cls in (Subclass, Mapping):
            events.clear()
            modules = cls([(name, value), ("after", None)])
            sys.modules = modules
            assert bootstrap._reorder_module(spec) is value
            assert events == [("pop", name), ("set", name, value)]
            assert list(modules if cls is Subclass else modules.data) == ["after", name]

        rebound = Subclass()
        new_name = "_atomic_reorder_rebound"

        class Rebinding(Subclass):
            def pop(self, key):
                result = super().pop(key)
                sys.modules = rebound
                spec.name = new_name
                return result

        events.clear()
        modules = Rebinding({name: value})
        sys.modules = modules
        assert bootstrap._reorder_module(spec) is value
        assert events == [("pop", name), ("set", new_name, value)]
        assert sys.modules is rebound
        assert rebound[new_name] is value
        assert name not in modules

        failure = KeyError("custom pop exception")

        class Raising(Subclass):
            def pop(self, key):
                raise failure

        events.clear()
        sys.modules = Raising({new_name: value})
        raises(KeyError, lambda: bootstrap._reorder_module(spec), failure)
        assert events == []
    finally:
        sys.modules = original


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


source = source_bootstrap()
for bootstrap in (frozen, source):
    if sys.implementation.name == "rustpython":
        # Run this before inspecting the new helper so a baseline executable
        # fails on the actual pop/set gap, not just a missing private API.
        modes = ("load", "exec")
        if hasattr(bootstrap, "_load_backward_compatible"):
            modes += ("legacy", "legacy_error")
        for mode in modes:
            check_trace_presence(bootstrap, mode)
    if hasattr(bootstrap, "_reorder_module"):
        check_custom_mappings(bootstrap)

if sys.implementation.name == "rustpython":
    check_native_helper()
