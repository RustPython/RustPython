def foo():
    a = 5
    return 10 + a


def bar():
    a = 1e6
    return a / 5.0


def baz(a: int, b: int):
    return a + b + 12


def tests():
    assert foo() == 15
    assert bar() == 2e5
    assert baz(17, 20) == 49
    assert baz(17, 22.5) == 51.5


tests()

if hasattr(foo, "__jit__"):
    print("Has jit")
    foo.__jit__()
    bar.__jit__()
    baz.__jit__()
    tests()

    # A sum that stops fitting in a machine word is not an error: the compiled
    # code hands its operands back and the interpreter answers with a bignum.
    def add(a: int, b: int) -> int:
        return a + b

    add.__jit__()
    assert add(1, 2) == 3
    assert add(2**62, 2**62) == 2**63
