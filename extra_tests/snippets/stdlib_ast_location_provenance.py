"""Explicit AST locations survive preprocessing and AST output conversion."""

import ast


def set_location(node, line, col, end_col):
    node.lineno = node.end_lineno = line
    node.col_offset = col
    node.end_col_offset = end_col
    return node


# Outbound conversion must not merge or drop explicitly supplied literal nodes.
for optimize in (0, 1, 2):
    for flags in (ast.PyCF_ONLY_AST, ast.PyCF_ONLY_AST | ast.PyCF_OPTIMIZED_AST):
        values = [
            set_location(ast.Constant(value=s), 1, -1, -1) for s in ("a", "", "b")
        ]
        joined = set_location(ast.JoinedStr(values=values), 1, 0, 8)
        result = compile(
            ast.Expression(body=joined), "<locations>", "eval", flags, optimize=optimize
        )
        assert [node.value for node in result.body.values] == ["a", "", "b"]
        assert all(
            node.col_offset == node.end_col_offset == -1 for node in result.body.values
        )

# The replacement Pass derives from the statement, not its string value.
tree = ast.parse('def f():\n    "doc"\n')
set_location(tree.body[0].body[0], 2, -1, -1)
set_location(tree.body[0].body[0].value, 2, 4, 9)
result = compile(
    tree, "<locations>", "exec", ast.PyCF_ONLY_AST | ast.PyCF_OPTIMIZED_AST, optimize=2
)
replacement = result.body[0].body[0]
assert isinstance(replacement, ast.Pass)
assert (
    replacement.lineno,
    replacement.end_lineno,
    replacement.col_offset,
    replacement.end_col_offset,
) == (2, 2, -1, 3)

# The optimizer's generated format spec is an artificial Constant; the argument
# and the outer expression retain their own distinct source locations.
tree = ast.parse("'%3s' % (value,)", mode="eval")
set_location(tree.body, 1, -1, -1)
set_location(tree.body.right.elts[0], 1, 8, 13)
result = compile(
    tree, "<locations>", "eval", ast.PyCF_ONLY_AST | ast.PyCF_OPTIMIZED_AST, optimize=0
)
formatted = result.body.values[0]
assert (result.body.col_offset, result.body.end_col_offset) == (-1, -1)
assert (formatted.col_offset, formatted.end_col_offset) == (8, 13)
assert isinstance(formatted.format_spec, ast.Constant)
assert formatted.format_spec.value == ">3"
spec = formatted.format_spec
assert (spec.lineno, spec.end_lineno, spec.col_offset, spec.end_col_offset) == (
    -1,
    -1,
    -1,
    -1,
)
