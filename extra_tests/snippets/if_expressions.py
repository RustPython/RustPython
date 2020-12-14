def ret(expression):
    return expression


assert ret("0" if True else "1") == "0"
assert ret("0" if False else "1") == "1"

assert ret("0" if False else ("1" if True else "2")) == "1"
assert ret("0" if False else ("1" if False else "2")) == "2"

assert ret(("0" if True else "1") if True else "2") == "0"
assert ret(("0" if False else "1") if True else "2") == "1"

a = True
b = False
assert ret("0" if a or b else "1") == "0"
assert ret("0" if a and b else "1") == "1"


def func1():
    return 0

def func2():
    return 20

assert ret(func1() or func2()) == 20

a, b = (1, 2) if True else (3, 4)
assert a == 1
assert b == 2
