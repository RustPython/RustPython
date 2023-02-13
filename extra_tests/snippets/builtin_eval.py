assert 3 == eval('1+2')

code = compile('5+3', 'x.py', 'eval')
assert eval(code) == 8
