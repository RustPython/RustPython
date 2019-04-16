
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
        nonlocal b
        b = 3
    b = 2
    y()
    return b

res = x()
assert res == 3, str(res)
