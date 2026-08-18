"""Module/class-scope locals() must not corrupt or leak __conditional_annotations__.

CPython's _PyFrame_GetLocals never syncs cell variables into a module/class
scope's namespace dict, it just returns the dict directly (verified against
CPython 3.14.6). __conditional_annotations__ is a cell in both scopes, but
only module codegen also writes it into the dict (StoreName); class codegen
only ever uses the cell (StoreDeref). So it's visible via locals()/dir() at
module scope and absent at class scope.

RustPython's fast-locals-to-mapping sync used to read every cellvar's value
straight from the cell regardless of scope. At module scope the cell is
always empty, so this overwrote the dict's real value with None -- deleting
it, and the next annotated statement raised NameError. At class scope it
leaked __conditional_annotations__ into locals()/dir(), which CPython never
does.
"""

count: int = 1
_ = locals()
maybe: int = None  # used to raise NameError before the fix
assert maybe is None

assert "__conditional_annotations__" in dir(), (
    "module-level annotation should expose __conditional_annotations__, matching CPython"
)

exec("a: int = 1\nlocals()\nb: int = 2")

if True:
    x: int = 1
vars()
if True:
    y: int = 2
assert (x, y) == (1, 2)


class C:
    if True:
        cx: int = 1
    locals()
    if True:
        cy: int = 2
    assert "__conditional_annotations__" not in dir(), (
        "class-level locals() should not leak __conditional_annotations__, matching CPython"
    )


assert (C.cx, C.cy) == (1, 2)

print("ok")
