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
