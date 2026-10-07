import dis
import sys
import types

from testutils import assert_raises

## BinaryOp inplace-add unicode: deopt falls back to __add__/__iadd__


class S(str):
    def __add__(self, other):
        return "ADD"

    def __iadd__(self, other):
        return "IADD"


def add_path_fallback_uses_add():
    x = "a"
    y = "b"
    for i in range(1200):
        if i == 600:
            x = S("s")
            y = "t"
        x = x + y
    return x


def iadd_path_fallback_uses_iadd():
    x = "a"
    y = "b"
    for i in range(1200):
        if i == 600:
            x = S("s")
            y = "t"
        x += y
    return x


assert add_path_fallback_uses_add().startswith("ADD")
assert iadd_path_fallback_uses_iadd().startswith("IADD")


## BINARY_SUBSCR_STR_INT: ASCII singleton identity


def check_ascii_subscr_singleton_after_warmup():
    s = "abc"
    first = None
    for i in range(4000):
        c = s[0]
        if i >= 3500:
            if first is None:
                first = c
            else:
                assert c is first


check_ascii_subscr_singleton_after_warmup()


## BINARY_SUBSCR_STR_INT: Latin-1 singleton identity


def check_latin1_subscr_singleton_after_warmup():
    for s in ("abc", "éx"):
        first = None
        for i in range(5000):
            c = s[0]
            if i >= 4500:
                if first is None:
                    first = c
                else:
                    assert c is first


check_latin1_subscr_singleton_after_warmup()


## LOAD_ATTR_METHOD_WITH_VALUES: keys-version shadow check


class MethodHolder:
    def m(self):
        return "method"


def method_shadowed_after_specialization():
    obj = MethodHolder()
    obj.pad = 1
    for _ in range(300):
        assert obj.m() == "method"
    # Shadowing after warmup must deopt the stamp-based shadow skip.
    obj.m = lambda: "instance"
    assert obj.m() == "instance"
    del obj.m
    assert obj.m() == "method"
    obj.__dict__["m"] = lambda: "dict"
    assert obj.m() == "dict"
    del obj.__dict__["m"]
    assert obj.m() == "method"


method_shadowed_after_specialization()


def method_with_value_only_updates():
    obj = MethodHolder()
    obj.pad = 0
    for i in range(500):
        obj.pad = i  # value-only update keeps the keys-version stamp
        assert obj.m() == "method"


method_with_value_only_updates()


## LOAD_ATTR_WITH_HINT / STORE_ATTR: entry-index hint invalidation


class Plain:
    pass


def load_hint_survives_key_churn():
    obj = Plain()
    obj.a = 1
    obj.b = 2
    obj.x = "first"
    for _ in range(300):
        assert obj.x == "first"
    del obj.a
    del obj.b
    assert obj.x == "first"
    del obj.x
    try:
        obj.x
    except AttributeError:
        pass
    else:
        raise AssertionError("expected AttributeError")
    obj.x = "second"
    assert obj.x == "second"


load_hint_survives_key_churn()


def store_hint_survives_dict_replacement():
    obj = Plain()
    obj.v = 0
    for i in range(500):
        obj.v = i
        assert obj.v == i
    obj.__dict__ = {"v": "fresh"}
    for i in range(300):
        obj.v = i
        assert obj.v == i
    obj.__dict__.clear()
    obj.v = "back"
    assert obj.v == "back"


store_hint_survives_dict_replacement()


## Shared shape stamps: same-layout instances share the shadow-check stamp


class ShapedCounter:
    def __init__(self):
        self.a = 1
        self.b = 2

    def m(self):
        return "method"


def shape_sharing_shadow_one_instance():
    objs = [ShapedCounter() for _ in range(30)]
    for _ in range(300):
        for o in objs:
            assert o.m() == "method"
    objs[13].m = lambda: "thirteen"
    for i, o in enumerate(objs):
        expected = "thirteen" if i == 13 else "method"
        assert o.m() == expected
    del objs[13].m
    for o in objs:
        assert o.m() == "method"


shape_sharing_shadow_one_instance()


def holey_dict_falls_back():
    class Holey:
        def m(self):
            return "g"

    def call_m(obj):
        # single LOAD_ATTR cache site shared by both instances
        return obj.m()

    g1, g2 = Holey(), Holey()
    for o in (g1, g2):
        o.x = 1
        o.y = 2
        o.z = 3
        del o.y  # leaves a hole in the entries
    for _ in range(300):
        assert call_m(g1) == "g"
        assert call_m(g2) == "g"
    g2.m = lambda: "g2"
    assert call_m(g1) == "g"
    assert call_m(g2) == "g2"


holey_dict_falls_back()


## LOAD_ATTR_MODULE: cached entries and generic-lookup guards


def module_attribute_cache_guards():
    module = types.ModuleType("cached_module")
    module.x = 1
    module.f = lambda: 10
    # CPython's module cache bypasses these descriptors when already shadowed.
    if sys.implementation.name == "rustpython":
        module.__dict__.update(__class__="shadow", __dict__="shadow")

    def read():
        return module.x

    def call():
        return module.f()

    def name():
        return module.__name__

    def cls():
        return module.__class__

    def namespace():
        return module.__dict__

    for _ in range(300):
        assert read() == 1
        assert call() == 10
        assert name() == "cached_module"
        assert cls() is types.ModuleType
        assert namespace() is vars(module)
    for func in (read, call, name):
        assert any(
            op.opname == "LOAD_ATTR_MODULE"
            for op in dis.get_instructions(func, adaptive=True)
        )

    module.x = 2
    module.f = lambda: 11
    assert read() == 2
    assert call() == 11
    del module.x
    with assert_raises(AttributeError):
        read()
    module.x = 3
    assert read() == 3

    module.__getattr__ = lambda attr: "hook"
    del module.x
    assert read() == "hook"
    del module.__getattr__
    module.x = 4
    for _ in range(300):
        assert read() == 4
    module.__dict__["__name__"] = "renamed"
    assert name() == "renamed"
    module.__dict__["__class__"] = 5
    module.__dict__["__dict__"] = 6
    assert cls() is types.ModuleType
    assert namespace() is vars(module)
    for i in range(200):
        setattr(module, f"k{i}", i)
        assert read() == 4

    module.__dict__.clear()
    module.__dict__.update(x=5, f=lambda: 12)
    assert read() == 5
    assert call() == 12

    # Replace the actual binding used at the warmed cache site.
    module_name = "_module_attr_specialization_test"
    previous = sys.modules.get(module_name)
    replacement = types.ModuleType(module_name)
    replacement.padding = None
    replacement.x = 6
    try:
        sys.modules[module_name] = replacement
        module = __import__(module_name)
        assert read() == 6
    finally:
        if previous is None:
            del sys.modules[module_name]
        else:
            sys.modules[module_name] = previous

    class CustomModule(types.ModuleType):
        @property
        def x(self):
            return "descriptor"

    module.__class__ = CustomModule
    assert read() == "descriptor"


module_attribute_cache_guards()


def module_attribute_custom_dict_key():
    class Key:
        matches = True

        def __hash__(self):
            return hash("x")

        def __eq__(self, other):
            return self.matches and other == "x"

    module = types.ModuleType("custom_key")
    key = Key()
    module.__dict__[key] = 1

    def read():
        return module.x

    for _ in range(300):
        assert read() == 1
    key.matches = False
    with assert_raises(AttributeError):
        read()


module_attribute_custom_dict_key()


## STORE_SUBSCR_LIST_INT: the replaced element's finalizer can read the list


def store_subscr_list_int_finalizer_reads_list():
    target = [None]
    seen = []

    class Item:
        def __init__(self, value):
            self.value = value

        def __del__(self):
            seen.append((self.value, len(target), target[0] is self))

    def store(value):
        target[0] = value

    for i in range(300):
        store(Item(i))
    assert any(
        op.opname == "STORE_SUBSCR_LIST_INT"
        for op in dis.get_instructions(store, adaptive=True)
    )
    store(None)
    assert seen == [(i, 1, False) for i in range(300)]


store_subscr_list_int_finalizer_reads_list()
