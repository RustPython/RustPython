from testutils import assertRaises

# Test global and nonlocal funkyness

a = 2

def b():
    global a
    a = 4

assert a == 2
b()
assert a == 4


def x():
    def y():
        global a
        nonlocal b
        assert a == 4, a
        b = 3
    a = "no!" # a here shouldn't be seen by the global above.
    b = 2
    y()
    return b

res = x()
assert res == 3, str(res)

# Invalid syntax:
src = """
b = 2
global b
"""

with assertRaises(SyntaxError):
    exec(src)

# Invalid syntax:
src = """
nonlocal c
"""

with assertRaises(SyntaxError):
    exec(src)


# Invalid syntax:
src = """
def f():
    def x():
        nonlocal c
c = 2
"""

with assertRaises(SyntaxError):
    exec(src)

# Invalid syntax:
src = """
def a():
    nonlocal a
"""

with assertRaises(SyntaxError):
    exec(src)

# class X:
#     nonlocal c
#     c = 2

