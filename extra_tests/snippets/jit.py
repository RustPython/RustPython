def foo() -> int:
    a = 5
    return 10 + a


def bar() -> float:
    a = 1e6
    return a / 5.0


def baz(a: int, b: float) -> float:
    return a + b + 12


def tuple_identity(t: tuple) -> tuple:
    return t


def mixed_args(a: int, b: float, c: tuple) -> tuple:
    return a, b, c


def fib(n: int) -> int:
    if n == 0 or n == 1:
        return 1
    return fib(n - 1) + fib(n - 2)


def tests():
    test_funcs = [foo, bar, baz, baz, tuple_identity, mixed_args, fib]
    test_args = [
        [],
        [],
        [17, 20],
        [17, 22.5],
        [(1, 2)],
        [1, 3.5, (1, 2)],
        [5],
    ]
    test_expected = [
        15,
        2e5,
        49,
        51.5,
        (1, 2),
        (1, 3.5, (1, 2)),
        8,
    ]
    for f, args, expected in zip(test_funcs, test_args, test_expected):
        assert f(*args) == expected
        print(f"Test {f.__name__}({', '.join(map(str, args))}) == {expected} PASSED")


tests()

if hasattr(foo, "__jit__"):
    print("Has jit, JIT test start:")
    foo.__jit__()
    bar.__jit__()
    baz.__jit__()
    tuple_identity.__jit__()
    fib.__jit__()
    tests()
