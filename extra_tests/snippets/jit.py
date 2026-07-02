def foo() -> int:
    a = 5
    return 10 + a


def bar() -> float:
    a = 1e6
    return a / 5.0


def baz(a: float, b: float) -> float:
    return a + b + 12

def tuple_identity(t: tuple) -> tuple:
    return t


def tests():
    assert foo() == 15
    assert bar() == 2e5
    assert baz(17, 20) == 49
    assert baz(17, 22.5) == 51.5

    tupleId = (1, 2)
    assert tuple_identity(tupleId) == tupleId


tests()

if hasattr(foo, "__jit__"):
    print("Has jit")
    foo.__jit__()
    bar.__jit__()
    baz.__jit__()
    tuple_identity.__jit__()
    tests()
