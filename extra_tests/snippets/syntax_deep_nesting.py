import ast

from testutils import assert_raises

# Deeply nested source used to exhaust the native stack, killing the process
# with a signal no `except` here could catch. What is asserted below is mostly
# that this file finishes at all: reaching the end means each case raised a
# Python exception instead of taking the interpreter down.
#
# Two limits keep that from happening, and each has to apply before a tree that
# deep is built. The parser grows its own stack when it runs low on one, and a
# tree it returns costs stack both to walk and to drop, so checking afterwards
# is already too late.


def nested_binop(depth):
    return "x" + "+1" * depth


def nested_brackets(depth):
    return "[" * depth + "1" + "]" * depth


# Brackets are counted in the source text before it is parsed, the way CPython
# counts them in its tokenizer, so no tree is built for these at all.
assert_raises(SyntaxError, compile, nested_brackets(200000), "<test>", "exec")
assert_raises(SyntaxError, ast.parse, nested_brackets(200000))
assert_raises(SyntaxError, compile, nested_brackets(5000), "<test>", "exec")
assert_raises(SyntaxError, ast.parse, nested_brackets(5000))

# Nesting that reaches the parser is bounded by the compiler's recursion limit,
# applied to the tree it returns. CPython bounds the same walks by how much
# stack is left rather than by a count, so it accepts depths rejected here;
# either outcome is fine, crashing is not.
try:
    compile(nested_binop(20000), "<test>", "exec")
except RecursionError:
    pass

# `compile(..., PyCF_ONLY_AST)`, which `ast.parse` uses, returns before the
# symbol table runs, so it used to reach the limit of no pass at all.
try:
    ast.parse(nested_binop(5000))
except RecursionError:
    pass

# Nesting within both limits still compiles and runs.
assert compile(nested_binop(100), "<test>", "exec")
assert compile(nested_brackets(100), "<test>", "exec")
assert ast.parse(nested_binop(100))
assert eval(compile(nested_binop(100), "<test>", "eval"), {"x": 0}) == 100
