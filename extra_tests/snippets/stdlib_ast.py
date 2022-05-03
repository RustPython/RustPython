
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
assert n.body[0].name == 'foo'
foo = n.body[0]
assert foo.lineno == 2
print(foo.body)
assert len(foo.body) == 2
print(foo.body[0])
print(foo.body[0].value.func.id)
assert foo.body[0].value.func.id == 'print'
assert foo.body[0].lineno == 3
assert foo.body[1].lineno == 4

n = ast.parse("3 < 4 > 5\n")
assert n.body[0].value.left.value == 3
assert 'Lt' in str(n.body[0].value.ops[0])
assert 'Gt' in str(n.body[0].value.ops[1])
assert n.body[0].value.comparators[0].value == 4
assert n.body[0].value.comparators[1].value == 5


n = ast.parse('from ... import a\n')
print(n)
i = n.body[0]
assert i.level == 3
assert i.module is None
assert i.names[0].name == 'a'
assert i.names[0].asname is None

