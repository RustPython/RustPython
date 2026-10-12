import ast
import copy

print(ast)

source = """
def foo():
    print('bar')
    pass
"""
n = ast.parse(source)
print(n)
print(n.body)
print(n.body[0].name)
assert n.body[0].name == "foo"
foo = n.body[0]
assert foo.lineno == 2
print(foo.body)
assert len(foo.body) == 2
print(foo.body[0])
print(foo.body[0].value.func.id)
assert foo.body[0].value.func.id == "print"
assert foo.body[0].lineno == 3
assert foo.body[1].lineno == 4

n = ast.parse("3 < 4 > 5\n")
assert n.body[0].value.left.value == 3
assert "Lt" in str(n.body[0].value.ops[0])
assert "Gt" in str(n.body[0].value.ops[1])
assert n.body[0].value.comparators[0].value == 4
assert n.body[0].value.comparators[1].value == 5


n = ast.parse("from ... import a\n")
print(n)
i = n.body[0]
assert i.level == 3
assert i.module is None
assert i.names[0].name == "a"
assert i.names[0].asname is None


# Regression: parsed AST identifier fields are interned, matching CPython.
name_literal = "x"
name = ast.parse("x").body[0].value
assert name.id is name_literal

name.extra = object()
replacement = copy.replace(name)
assert replacement.id is name.id
assert replacement.ctx is name.ctx
assert not hasattr(replacement, "extra")

function_name = "f"
function = ast.parse("def f(): pass").body[0]
assert function.name is function_name

async_function = ast.parse("async def f(): pass").body[0]
assert async_function.name is function_name


# Regression test for issue #4862:
# A cyclic AST fed to compile() used to overflow the Rust stack and SIGSEGV.
# After the fix, the recursion guard in ast_from_object raises RecursionError,
# matching CPython's behavior. Covers both Box<T> descents (UnaryOp, BinOp,
# Call, Attribute) and Vec<T> descents (List, Tuple).
import warnings


def _cyclic_cases():
    # Box<Expr> descents
    u = ast.UnaryOp(op=ast.Not(), operand=None, lineno=0, col_offset=0)
    u.operand = u
    yield "UnaryOp", u

    b = ast.BinOp(
        op=ast.Add(),
        left=None,
        right=ast.Constant(value=0, lineno=0, col_offset=0),
        lineno=0,
        col_offset=0,
    )
    b.left = b
    yield "BinOp", b

    c = ast.Call(func=None, args=[], keywords=[], lineno=0, col_offset=0)
    c.func = c
    yield "Call", c

    a = ast.Attribute(value=None, attr="x", ctx=ast.Load(), lineno=0, col_offset=0)
    a.value = a
    yield "Attribute", a

    # Vec<Expr> descents
    lst = ast.List(ctx=ast.Load(), lineno=0, col_offset=0)
    lst.elts = [lst]
    yield "List", lst

    tup = ast.Tuple(ctx=ast.Load(), lineno=0, col_offset=0)
    tup.elts = [tup]
    yield "Tuple", tup


with warnings.catch_warnings():
    warnings.simplefilter("ignore")
    for name, node in _cyclic_cases():
        try:
            compile(ast.Expression(node), "<cyclic>", "eval")
            raise AssertionError(f"cyclic {name} should raise RecursionError")
        except RecursionError:
            pass  # expected; matches CPython


# Python 3.15 rejects omitted required fields and unknown constructor keywords.
from testutils import assert_raises

with assert_raises(TypeError) as exc:
    ast.Name()
assert (
    str(exc.exception)
    == "ast.Name.__init__ missing 1 required positional argument: 'id'"
)

with assert_raises(TypeError) as exc:
    ast.BinOp()
assert str(exc.exception) == (
    "ast.BinOp.__init__ missing 3 required positional arguments: 'left', 'op', and 'right'"
)

with assert_raises(TypeError) as exc:
    ast.BinOp(op=ast.Add())
assert str(exc.exception) == (
    "ast.BinOp.__init__ missing 2 required positional arguments: 'left' and 'right'"
)

with assert_raises(TypeError) as exc:
    ast.Name(id="x", extra=True)
assert (
    str(exc.exception) == "ast.Name.__init__ got an unexpected keyword argument 'extra'"
)

with assert_raises(TypeError) as exc:
    ast.Name("x", id="y")
assert str(exc.exception) == "ast.Name got multiple values for argument 'id'"

# Constructor defaults and explicit placeholders still work.
assert ast.Name("x").ctx is ast.Name("y").ctx
assert isinstance(ast.Name("x").ctx, ast.Load)
assert ast.FunctionDef(name="f", args=ast.arguments()).returns is None
assert ast.Module().body == []
assert ast.Module().body is not ast.Module().body
assert ast.BinOp(left=None, op=None, right=None).left is None


class FieldsWithoutTypes(ast.AST):
    _fields = ("value",)


assert not hasattr(FieldsWithoutTypes(), "value")
assert FieldsWithoutTypes(value=1).value == 1


class IncompleteFieldTypes(ast.AST):
    _fields = ("value",)
    _field_types = {}


with assert_raises(TypeError) as exc:
    IncompleteFieldTypes()
assert (
    str(exc.exception)
    == "Field 'value' is missing from IncompleteFieldTypes._field_types"
)


for key in ("a'b", "line\nbreak"):
    with assert_raises(TypeError) as exc:
        ast.Name(id="x", **{key: 1})
    assert str(exc.exception) == (
        f"ast.Name.__init__ got an unexpected keyword argument {key!r}"
    )
