
def foo():
    a = 5
    return 10 + a

def bar():
    a = 1e6
    return a / 5.0


def tests():
    assert foo() == 15
    assert bar() == 2e5

tests()

if hasattr(foo, "__jit__"):
    print("Has jit")
    foo.__jit__()
    tests()
