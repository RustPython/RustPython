""" This script can be used to test the equivalence in parsing between
rustpython and cpython.

Usage example:

$ python crawl_sourcecode.py crawl_sourcecode.py > cpython.txt
$ cargo run crawl_sourcecode.py crawl_sourcecode.py > rustpython.txt
$ diff cpython.txt rustpython.txt
"""


import ast
import sys
import symtable

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

def print_table(table, indent=0):
    print(' '*indent, 'table:', table.get_name())
    print(' '*indent, ' ', 'Syms:')
    for sym in table.get_symbols():
        print(' '*indent, '  ', sym.get_name(), 'is_referenced=', sym.is_referenced(), 'is_assigned=', sym.is_assigned())
    print(' '*indent, ' ', 'Child tables:')
    for child in table.get_children():
        print_table(child, indent=indent+shift)

table = symtable.symtable(source, 'a', 'exec')
print_table(table)
