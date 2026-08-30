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

    # A negative exponent and an answer too wide for a machine word both
    # send the operands back to the interpreter rather than answering 0 or
    # wrapping.
    def ipow(a: int, b: int) -> int:
        return a**b

    ipow.__jit__()
    assert ipow(2, 10) == 1024
    assert ipow(2, -2) == 0.25
    assert ipow(2, 33) == 8589934592

    # A huge-magnitude exponent used to trap the process outright instead of
    # deoptimizing; a finite base whose power overflows a double, a
    # zero base with a negative exponent, and a negative zero base all
    # deoptimize too, and the interpreter raises or answers exactly the way
    # it would without the JIT.
    def fpow(a: float, b: float) -> float:
        return a**b

    fpow.__jit__()
    assert fpow(2.0, 3.0) == 8.0
    assert fpow(-8.0, 2.0) == 64.0
    for base, exponent in [(2.0, 1e300), (-2.0, 1e300), (1e308, 2.0)]:
        try:
            fpow(base, exponent)
            raise AssertionError("expected OverflowError")
        except OverflowError:
            pass
    try:
        fpow(0.0, -1.0)
        raise AssertionError("expected ZeroDivisionError")
    except ZeroDivisionError:
        pass
    assert str(fpow(-0.0, 3.0)) == "-0.0"

    # Division by zero raises rather than returning an infinity.
    def fdiv(a: float, b: float) -> float:
        return a / b

    fdiv.__jit__()
    assert fdiv(4.0, 2.0) == 2.0
    try:
        fdiv(1.0, 0.0)
        raise AssertionError("expected ZeroDivisionError")
    except ZeroDivisionError:
        pass
