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

    # A guard that fires with several locals live and a partial expression on
    # the stack. The interpreter picks the addition up again with `total`, `n`
    # and `step` as the guard saw them and `n * step` already computed, so a
    # dropped local or a mis-ordered stack shows up as a wrong sum or an
    # UnboundLocalError rather than as a crash.
    def mixed(n: int, step: int) -> int:
        total = 0
        while n > 0:
            total = total + n * step
            n = n - 1
        return total

    mixed.__jit__()
    assert mixed(5, 1) == 15
    # `total` reaches 5 * 2**60 on the first iteration, which fits, and the
    # second adds 2**62 to it, which does not - so the guard fires two
    # iterations in, with the multiply's result already on the stack.
    assert mixed(5, 2**60) == 17293822569102704640

    # A frame that leaves because a *nested* frame gave up has no record of its
    # own: the record in the buffer belongs to the frame that wrote it.
    # `blow(1)` overflows on its own addition, and `blow(2)` reaches that
    # overflow one frame down - continuing the outer frame from the inner
    # frame's offset would add 2**62 to itself and answer 2**63.
    def blow(n: int) -> int:
        if n == 0:
            return 4611686018427387904
        return blow(n - 1) + blow(n - 1)

    blow.__jit__()
    assert blow(0) == 2**62
    assert blow(2) == 2**64

    # Depth rather than fidelity: the multiply overflows about forty frames
    # down, so forty callers each leave on a nested status instead of their own
    # guard. It cannot tell a right resume from a wrong one - `grow` is
    # tail-recursive, so what the outermost frame has left to do after the
    # guard is exactly what the deepest frame has left to do, and both answer
    # 3**50. `blow` above is what pins whose record gets honoured.
    def grow(n: int, acc: int) -> int:
        if n < 1:
            return acc
        return grow(n - 1, acc * 3)

    grow.__jit__()
    assert grow(50, 1) == 717897987691852588770249

    # A guard lists the locals the compiler had seen when it was lowered.
    # `extra` is stored further down the loop body, so the guard on the sum
    # cannot describe it - yet the backward jump means it is already bound by
    # the time that guard fires on the second iteration. A local a site cannot
    # describe is never read back after a resume, because a read the compiler
    # cannot prove bound is a LOAD_FAST_CHECK and no function containing one
    # compiles at all; it stays visible through `f_locals` though, so a site
    # that cannot describe every bound local restarts the call instead of
    # resuming without it.
    def late(n: int, step: int) -> int:
        total = 0
        while n > 0:
            total = total + n * step
            if n == 5:
                extra = 1
            n = n - 1
        return total // n

    late.__jit__()
    try:
        late(5, 2**60)
        raise AssertionError("expected ZeroDivisionError")
    except ZeroDivisionError as exc:
        frame = exc.__traceback__
        while frame.tb_next is not None:
            frame = frame.tb_next
        locals_at_raise = frame.tb_frame.f_locals
        assert locals_at_raise["total"] == 17293822569102704640, locals_at_raise
        assert locals_at_raise["extra"] == 1, locals_at_raise
