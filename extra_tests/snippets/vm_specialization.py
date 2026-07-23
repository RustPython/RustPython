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
