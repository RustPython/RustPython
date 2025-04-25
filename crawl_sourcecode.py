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
import dis

filename = sys.argv[1]
print('Crawling file:', filename)


with open(filename, 'r') as f:
    source = f.read()

t = ast.parse(source)
print(t)

shift = 3
def print_node(node, indent=0):
    indents = ' ' * indent
    if isinstance(node, ast.AST):
        lineno = 'row={}'.format(node.lineno) if hasattr(node, 'lineno') else ''
        print(indents, "NODE", node.__class__.__name__, lineno)
        for field in node._fields:
            print(indents,'-', field)
            f = getattr(node, field)
            if isinstance(f, list):
                for f2 in f:
                    print_node(f2, indent=indent+shift)
            else:
                print_node(f, indent=indent+shift)
    else:
        print(indents, 'OBJ', node)

print_node(t)

# print(ast.dump(t))
flag_names = [
    'is_referenced',
    'is_assigned',
    'is_global',
    'is_local',
    'is_parameter',
    'is_free',
]

def print_table(table, indent=0):
    indents = ' ' * indent
    print(indents, 'table:', table.get_name())
    print(indents, ' ', 'name:', table.get_name())
    print(indents, ' ', 'type:', table.get_type())
    print(indents, ' ', 'line:', table.get_lineno())
    print(indents, ' ', 'identifiers:', table.get_identifiers())
    print(indents, ' ', 'Syms:')
    for sym in table.get_symbols():
        flags = []
        for flag_name in flag_names:
            func = getattr(sym, flag_name)
            if func():
                flags.append(flag_name)
        print(indents, '   sym:', sym.get_name(), 'flags:', ' '.join(flags))
    if table.has_children():
        print(indents, ' ', 'Child tables:')
        for child in table.get_children():
            print_table(child, indent=indent+shift)

table = symtable.symtable(source, 'a', 'exec')
print_table(table)

print()
print('======== dis.dis ========')
print()
co = compile(source, filename, 'exec')
dis.dis(co)
