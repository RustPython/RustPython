
import ast
import sys

filename = sys.argv[1]
print('Crawling file:', filename)


with open(filename, 'r') as f:
    source = f.read()

t = ast.parse(source)
print(t)

shift = 3
def print_node(node, indent=0):
    if isinstance(node, ast.AST):
        print(' '*indent, "NODE", node.__class__.__name__)
        for field in node._fields:
            print(' '*indent,'-', field)
            f = getattr(node, field)
            if isinstance(f, list):
                for f2 in f:
                    print_node(f2, indent=indent+shift)
            else:
                print_node(f, indent=indent+shift)
    else:
        print(' '*indent, 'OBJ', node)

print_node(t)

# print(ast.dump(t))

