from typing import get_type_hints

from testutils import assert_raises

# Future f-string annotations canonicalize fields, not their literal padding.
for formatted, expected in (
    ("f'{ value }'", "f'{value}'"),
    ("f'{ value = }'", "f' value = {value!r}'"),
    ("f'{ value : >8}'", "f'{value: >8}'"),
    ("f'{ {1, 2} }'", "f'{ {1, 2}}'"),
):
    formatted_code = compile(
        "from __future__ import annotations\nx: " + formatted, "<test>", "exec"
    )
    assert expected in formatted_code.co_consts


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


# Conditional annotations belong to the inner class, not an enclosing scope.
for future in ("", "from __future__ import annotations\n"):
    namespace = {}
    exec(
        future
        + """
class Outer:
    class Inner:
        if True:
            included: int
        if False:
            omitted: str
""",
        namespace,
    )
    assert get_type_hints(namespace["Outer"].Inner) == {"included": int}

with assert_raises(SyntaxError):
    compile(
        "class C:\n    if True: x: int\n    global __conditional_annotations__",
        "<test>",
        "exec",
    )
