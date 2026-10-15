"""Compilation preserves missing AST columns separately from column zero."""

import ast
import dis


def store_positions(tree):
    code = compile(tree, "<ast-columns>", "exec").co_consts[0]
    return [
        tuple(instruction.positions)
        for instruction in dis.get_instructions(code)
        if instruction.opname == "STORE_FAST"
    ]


for column in (-1, -2, 0):
    tree = ast.parse("def f():\n x = 1\n")
    target = tree.body[0].body[0].targets[0]
    target.col_offset = target.end_col_offset = column
    expected = None if column < 0 else column
    assert store_positions(tree) == [(2, 2, expected, expected)]

# These two nodes have the same normalized navigation range but different
# semantic columns. Position metadata must therefore use node identity.
tree = ast.parse("def f():\n x = 1\n y = 2\n")
first, second = tree.body[0].body
for statement in (first, second):
    for node in ast.walk(statement):
        if hasattr(node, "lineno"):
            node.lineno = node.end_lineno = 2
first.targets[0].col_offset = first.targets[0].end_col_offset = 0
second.targets[0].col_offset = second.targets[0].end_col_offset = -1
assert store_positions(tree) == [(2, 2, 0, 0), (2, 2, None, None)]
