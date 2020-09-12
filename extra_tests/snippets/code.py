c1 = compile("1 + 1", "", 'eval')

code_class = type(c1)

def f(x, y, *args, power=1, **kwargs):
    print("Constant String", 2, None, (2, 4))
    assert code_class == type(c1)
    z = x * y
    return z ** power

c2 = f.__code__
# print(c2)
assert type(c2) == code_class
# print(dir(c2))
assert c2.co_argcount == 2
# assert c2.co_cellvars == ()
# assert isinstance(c2.co_code, bytes)
assert "Constant String" in c2.co_consts, c2.co_consts
print(c2.co_consts)
assert 2 in c2.co_consts, c2.co_consts
assert "code.py" in c2.co_filename
assert c2.co_firstlineno == 5, str(c2.co_firstlineno)
# assert isinstance(c2.co_flags, int) # 'OPTIMIZED, NEWLOCALS, NOFREE'
# assert c2.co_freevars == (), str(c2.co_freevars)
assert c2.co_kwonlyargcount == 1, (c2.co_kwonlyargcount)
# assert c2.co_lnotab == 0, c2.co_lnotab  # b'\x00\x01' # Line number table
assert c2.co_name == 'f', c2.co_name
# assert c2.co_names == ('code_class', 'type', 'c1', 'AssertionError'), c2.co_names # , c2.co_names
# assert c2.co_nlocals == 4, c2.co_nlocals #
# assert c2.co_stacksize == 2, 'co_stacksize',
# assert c2.co_varnames == ('x', 'y', 'power', 'z'), c2.co_varnames
