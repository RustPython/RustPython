
def foo():
    a = 5
    return 10 + a


assert foo() == 15

if hasattr(foo, "__jit__"):
    print("Has jit")
    foo.__jit__()
    assert foo() == 15
