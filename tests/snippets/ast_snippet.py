
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
