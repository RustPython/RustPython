from operator import imul, mul

from testutils import assert_raises

assert -3 // 2 == -2
assert -3 % 2 == 1

a = 4

assert a**3 == 64
assert a * 3 == 12
assert a / 2 == 2
assert 2 == a / 2
assert a % 3 == 1
assert a - 3 == 1
assert -a == -4
assert +a == 4

# bitwise

assert 8 >> 3 == 1
assert 8 << 3 == 64

# Left shift raises type error
assert_raises(TypeError, lambda: 1 << 0.1)
assert_raises(TypeError, lambda: 1 << "abc")

# Right shift raises type error
assert_raises(TypeError, lambda: 1 >> 0.1)
assert_raises(TypeError, lambda: 1 >> "abc")

# Left shift raises value error on negative
assert_raises(ValueError, lambda: 1 << -1)

# Right shift raises value error on negative
assert_raises(ValueError, lambda: 1 >> -1)

# Bitwise or, and, xor raises value error on incompatible types
assert_raises(TypeError, lambda: "abc" | True)
assert_raises(TypeError, lambda: "abc" & True)
assert_raises(TypeError, lambda: "abc" ^ True)
assert_raises(TypeError, lambda: True | "abc")
assert_raises(TypeError, lambda: True & "abc")
assert_raises(TypeError, lambda: True ^ "abc")
assert_raises(TypeError, lambda: "abc" | 1.5)
assert_raises(TypeError, lambda: "abc" & 1.5)
assert_raises(TypeError, lambda: "abc" ^ 1.5)
assert_raises(TypeError, lambda: 1.5 | "abc")
assert_raises(TypeError, lambda: 1.5 & "abc")
assert_raises(TypeError, lambda: 1.5 ^ "abc")
assert_raises(TypeError, lambda: True | 1.5)
assert_raises(TypeError, lambda: True & 1.5)
assert_raises(TypeError, lambda: True ^ 1.5)
assert_raises(TypeError, lambda: 1.5 | True)
assert_raises(TypeError, lambda: 1.5 & True)
assert_raises(TypeError, lambda: 1.5 ^ True)


def check_repeat_error(error, message, operation, *args):
    with assert_raises(error) as caught:
        operation(*args)
    assert str(caught.exception) == message, str(caught.exception)


class GetItemOnly:
    def __getitem__(self, index):
        raise IndexError


# An item slot does not imply support for repetition.
for value in (range(3), GetItemOnly()):
    name = type(value).__name__
    for operation, symbol in ((mul, "*"), (imul, "*=")):
        check_repeat_error(
            TypeError,
            f"unsupported operand type(s) for {symbol}: '{name}' and 'int'",
            operation,
            value,
            2,
        )
    check_repeat_error(
        TypeError,
        f"unsupported operand type(s) for *: 'int' and '{name}'",
        mul,
        2,
        value,
    )


# Native sequence slots use the same count conversion in both directions.
for value in ([1], (1,), "a", b"a", bytearray(b"a")):
    namespace = {}
    exec("def reverse(seq, count):\n    return count * seq", namespace)
    for operation_index, operation in enumerate((mul, namespace["reverse"], imul)):
        for _ in range(64):
            operation(value, 1)
        for count, error, message in (
            (1.5, TypeError, "can't multiply sequence by non-int of type 'float'"),
            (2**100, OverflowError, "cannot fit 'int' into an index-sized integer"),
        ):
            if (
                type(value) in (str, bytes, tuple)
                and operation_index == 1
                and error is OverflowError
            ):
                message = "Python int too large to convert to C ssize_t"
            check_repeat_error(error, message, operation, value, count)


class RepeatIndex:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


assert RepeatIndex(2) * [1] == [1, 1]
assert imul(2, [1]) == [1, 1]
check_repeat_error(
    TypeError,
    "unsupported operand type(s) for *=: 'RepeatIndex' and 'list'",
    imul,
    RepeatIndex(2),
    [1],
)
check_repeat_error(
    OverflowError,
    "cannot fit 'RepeatIndex' into an index-sized integer",
    mul,
    "a",
    RepeatIndex(-(2**100)),
)


# Each expression has fresh bytecode so cold and warmed paths are both checked.
for value in ("a", b"a", (1,), "", b"", ()):
    for expression in (
        "return seq * count",
        "return count * seq",
        "seq *= count; return seq",
        "count *= seq; return count",
    ):
        namespace = {}
        exec("def repeat(seq, count):\n    " + expression, namespace)
        repeat = namespace["repeat"]
        check_repeat_error(
            OverflowError,
            "cannot fit 'int' into an index-sized integer",
            repeat,
            value,
            2**100,
        )
        for _ in range(64):
            assert repeat(value, 2) == mul(value, 2)
        assert repeat(value, -2) == value[:0]
        assert repeat(value, 0) == value[:0]
        assert repeat(value, 1) == value
        for count in (2**100, -(2**100)):
            check_repeat_error(
                OverflowError,
                "Python int too large to convert to C ssize_t",
                repeat,
                value,
                count,
            )
        for other in ("a", b"a", (1,)):
            if type(other) is not type(value):
                check_repeat_error(
                    OverflowError,
                    "cannot fit 'int' into an index-sized integer",
                    repeat,
                    other,
                    2**100,
                )
        if expression == "count *= seq; return count":
            check_repeat_error(
                TypeError,
                f"unsupported operand type(s) for *=: 'RepeatIndex' and '{type(value).__name__}'",
                repeat,
                value,
                RepeatIndex(2),
            )
        else:
            assert repeat(value, RepeatIndex(2)) == mul(value, 2)
            check_repeat_error(
                OverflowError,
                "cannot fit 'RepeatIndex' into an index-sized integer",
                repeat,
                value,
                RepeatIndex(2**100),
            )
        subclass = type("RepeatSubclass", (type(value),), {})(value)
        assert repeat(subclass, 2) == mul(value, 2)
        overridden = type(
            "OverrideRepeat",
            (type(value),),
            {
                "__mul__": lambda self, count: "forward",
                "__rmul__": lambda self, count: "reverse",
            },
        )(value)
        expected = (
            "forward" if expression.startswith(("return seq", "seq *=")) else "reverse"
        )
        assert repeat(overridden, 2) == expected


class DeclineRepeat(GetItemOnly):
    calls = []

    def __mul__(self, other):
        self.calls.append("mul")
        return NotImplemented

    def __rmul__(self, other):
        self.calls.append("rmul")
        return NotImplemented

    def __imul__(self, other):
        self.calls.append("imul")
        return NotImplemented


# Numeric fallback must not invoke the same Python method through a sequence slot.
declining = DeclineRepeat()
for operation, args, expected in (
    (mul, (declining, 2), ["mul"]),
    (mul, (2, declining), ["rmul"]),
    (imul, (declining, 2), ["imul", "mul"]),
):
    DeclineRepeat.calls.clear()
    assert_raises(TypeError, operation, *args)
    assert DeclineRepeat.calls == expected


class RepeatList(list):
    pass


class RepeatInt(int):
    pass


assert "a" * RepeatInt(2) == RepeatInt(2) * "a" == "aa"
for method, operation in (
    ("__mul__", lambda value: value * 2),
    ("__rmul__", lambda value: 2 * value),
):
    value = RepeatList([1])
    assert operation(value) == [1, 1]
    setattr(RepeatList, method, lambda self, other: NotImplemented)
    assert_raises(TypeError, operation, value)
    delattr(RepeatList, method)
    assert operation(value) == [1, 1]
