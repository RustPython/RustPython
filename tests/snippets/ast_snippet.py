
import ast
print(ast)

source = """
def foo():
    print('bar')
"""
n = ast.parse(source)
print(n)
print(n.body)
print(n.body[0].name)
assert n.body[0].name == 'foo'
print(n.body[0].body)
print(n.body[0].body[0])
print(n.body[0].body[0].value.func.id)
assert n.body[0].body[0].value.func.id == 'print'
