
import ast

source = """
def foo():
    print('bar')
"""
n = ast.parse(source)
print(n)
