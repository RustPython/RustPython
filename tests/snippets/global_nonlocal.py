
# Test global and nonlocal funkyness

a = 2

def b():
    global a
    a = 4

assert a == 2
b()
assert a == 4

