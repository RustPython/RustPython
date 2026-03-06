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
