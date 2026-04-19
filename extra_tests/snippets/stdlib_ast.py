import ast

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


# Regression test for issue #4862:
# A cyclic AST fed to compile() used to overflow the Rust stack and SIGSEGV.
# After the fix, the recursion guard in ast_from_object raises RecursionError,
# matching CPython's behavior. Covers both Box<T> descents (UnaryOp, BinOp,
# Call, Attribute) and Vec<T> descents (List, Tuple).
import warnings


def _cyclic_cases():
    # Box<Expr> descents
    u = ast.UnaryOp(op=ast.Not(), lineno=0, col_offset=0)
    u.operand = u
    yield "UnaryOp", u

    b = ast.BinOp(
        op=ast.Add(),
        right=ast.Constant(value=0, lineno=0, col_offset=0),
        lineno=0,
        col_offset=0,
    )
    b.left = b
    yield "BinOp", b

    c = ast.Call(args=[], keywords=[], lineno=0, col_offset=0)
    c.func = c
    yield "Call", c

    a = ast.Attribute(attr="x", ctx=ast.Load(), lineno=0, col_offset=0)
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
