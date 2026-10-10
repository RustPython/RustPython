from typing import get_type_hints

def func(s: str) -> int:
    return int(s)

hints = get_type_hints(func)

# The order of type hints matters for certain functions
# e.g. functools.singledispatch
assert list(hints.items()) == [('s', str), ('return', int)]


# Template format specs use canonical expression text inside future annotations.
for template, expected in (
    ("t'{value:04}'", "t'{value:04}'"),
    ("t'{value:{(width)}}'", "t'{value:{width}}'"),
    ("t'{(value):{(width)!r}}'", "t'{(value):{width!r}}'"),
):
    annotation_code = compile(
        "from __future__ import annotations\nx: " + template, "<test>", "exec"
    )
    assert expected in annotation_code.co_consts
