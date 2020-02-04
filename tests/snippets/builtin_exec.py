exec("def square(x):\n return x * x\n")
assert 16 == square(4)  # noqa: F821

d = {}
exec("def square(x):\n return x * x\n", {}, d)
assert 16 == d['square'](4)

exec("assert 2 == x", {}, {'x': 2})
exec("assert 2 == x", {'x': 2}, {})
exec("assert 4 == x", {'x': 2}, {'x': 4})

exec("assert max(1, 2) == 2", {}, {})

exec("assert max(1, 5, square(5)) == 25", None)

# Local environment shouldn't replace global environment:
exec("assert max(1, 5, square(5)) == 25", None, {})

# Closures aren't available if local scope is replaced:
def g():
    seven = "seven"
    def f():
        try:
            exec("seven", None, {})
        except NameError:
            pass
        else:
            raise NameError("seven shouldn't be in scope")
    f()
g()

try:
    exec("", 1)
except TypeError:
    pass
else:
    raise TypeError("exec should fail unless globals is a dict or None")

g = globals()
g['x'] = 2
exec('x += 2')
assert x == 4  # noqa: F821
assert g['x'] == x  # noqa: F821

exec("del x")
assert 'x' not in g

assert 'g' in globals()
assert 'g' in locals()
exec("assert 'g' in globals()")
exec("assert 'g' in locals()")
exec("assert 'g' not in globals()", {})
exec("assert 'g' not in locals()", {})

del g

def f():
    g = 1
    assert 'g' not in globals()
    assert 'g' in locals()
    exec("assert 'g' not in globals()")
    exec("assert 'g' in locals()")
    exec("assert 'g' not in globals()", {})
    exec("assert 'g' not in locals()", {})

f()
