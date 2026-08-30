import sys


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
    # the time that guard fires on the second iteration. A site that cannot
    # describe every bound local restarts the call instead of resuming
    # without it.
    #
    # This reads the dropped local out of a traceback rather than out of the
    # function, and it has to. The obvious version - `return total + extra` -
    # cannot fail, because a read the compiler cannot prove bound compiles to
    # LOAD_FAST_CHECK, which has no lowering, so no function that would
    # observe the drop that way compiles in the first place. What does still
    # see it is anything reading the frame's fastlocals other than a
    # LOAD_FAST: `f_locals` here, a tracer stepping the resumed frame, a
    # debugger stopped in it.
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

    # The point of all of the above: compiling a function must not change
    # what it answers. Each `check` execs its own pair of functions so that
    # `__jit__()` on the compiled one cannot affect the interpreted one.
    def check(source, *args):
        """Assert that compiling a function does not change what it answers."""
        interpreted_scope = {}
        exec(source, interpreted_scope)
        compiled_scope = {}
        exec(source, compiled_scope)
        compiled_scope["f"].__jit__()

        def call(f):
            try:
                return ("value", f(*args))
            except Exception as e:
                return ("raised", type(e))

        expected = call(interpreted_scope["f"])
        actual = call(compiled_scope["f"])
        assert expected == actual, (source.strip(), args, expected, actual)
        if expected[0] == "value":
            assert type(expected[1]) is type(actual[1]), (
                source.strip(),
                args,
                type(expected[1]),
                type(actual[1]),
            )

    FLOOR = "def f(a: int, b: int) -> int:\n    return a // b\n"
    check(FLOOR, -7, 2)
    check(FLOOR, 7, -2)
    check(FLOOR, -7, -2)
    check(FLOOR, 1, 0)
    check(FLOOR, -(2**63), -1)

    MOD = "def f(a: int, b: int) -> int:\n    return a % b\n"
    check(MOD, -7, 2)
    check(MOD, 7, -2)
    check(MOD, 1, 0)

    MUL = "def f(a: int, b: int) -> int:\n    return a * b\n"
    check(MUL, 2**62, 4)
    check(MUL, 3, 4)

    IPOW = "def f(a: int, b: int) -> int:\n    return a ** b\n"
    check(IPOW, 2, -2)
    check(IPOW, 2, 64)
    check(IPOW, 2, 10)

    DIV = "def f(a: int, b: int) -> float:\n    return a / b\n"
    check(DIV, 1, 0)
    check(DIV, 2**60 + 1, 3)
    check(DIV, 7, 2)

    FDIV = "def f(a: float, b: float) -> float:\n    return a / b\n"
    check(FDIV, 1.0, 0.0)
    check(FDIV, 4.0, 2.0)

    FPOW = "def f(a: float, b: float) -> float:\n    return a ** b\n"
    check(FPOW, -8.0, 0.5)
    check(FPOW, 0.0, -1.0)
    check(FPOW, -8.0, 2.0)

    SHIFT = "def f(a: int, b: int) -> int:\n    return a << b\n"
    check(SHIFT, 1, 62)
    check(SHIFT, 1, 63)
    check(SHIFT, 1, 64)
    check(SHIFT, 1, -1)

    RSHIFT = "def f(a: int, b: int) -> int:\n    return a >> b\n"
    check(RSHIFT, -8, 1)
    check(RSHIFT, 1, 64)
    check(RSHIFT, 1, -1)

    ADD = "def f(a: int, b: int) -> int:\n    return a + b\n"
    check(ADD, 2**62, 2**62)
    check(ADD, 7, 3)

    # `Subtract`'s own arm calls `compile_sub(a, b, ...)` in call-site order; `NEG`
    # below reaches the same helper through a separate arm, `compile_sub(zero, a,
    # ...)`, with different operand order and arity. Covering both closes the gap
    # a fix in one arm and not the other would leave open.
    SUB = "def f(a: int, b: int) -> int:\n    return a - b\n"
    check(SUB, -(2**63), 1)
    check(SUB, 7, 3)

    NEG = "def f(a: int) -> int:\n    return -a\n"
    check(NEG, 7)
    check(NEG, -(2**63))

    # `fib_iter(95)` overflows a signed 64-bit accumulator partway through,
    # so the compiled run deoptimizes mid-loop and the interpreter finishes
    # it. Before this, the same call killed the process with SIGILL.
    def fib_iter(n: int) -> int:
        a = 0
        b = 1
        i = 0
        while i < n:
            a, b = b, a + b
            i = i + 1
        return a

    assert fib_iter(95) == 31940434634990099905
    fib_iter.__jit__()
    assert fib_iter(10) == 55
    assert fib_iter(95) == 31940434634990099905

    # A self-call compiles only under `Safety::Permissive`, which is what
    # `__jit__()` asks for. Automatic compilation uses `Strict`, which turns
    # a self-call down because the interpreter re-reads the global on every
    # call and rebinding the name has to stay observable (`instructions.rs`,
    # the `LoadGlobal` arm).
    def fib(n: int) -> int:
        if n < 2:
            return n
        return fib(n - 1) + fib(n - 2)

    fib.__jit__()
    assert fib(25) == 75025

    # A recursive call with more than one argument: the compiler collects
    # the arguments by popping, which walks them backwards, so this is the
    # shape that catches them arriving in the wrong order. `countdown(3, 1)`
    # is asymmetric in its two parameters, so passing them the wrong way
    # round gives a different answer rather than the same one.
    def countdown(a: int, b: int) -> int:
        if a < 1:
            return b
        return countdown(a - 1, b * 2)

    countdown.__jit__()
    assert countdown(3, 1) == 8
    assert countdown(0, 5) == 5

    # Compiled code runs no frame, so it reports no call, no line and no
    # return. A tracer installed while a function is compiled has to send
    # the call back to the interpreter, or it observes nothing at all: this
    # asserted an empty event list before the tracing test moved above the
    # compiled entry. `sys.monitoring` is checked too because it is a
    # separate switch that the same entry has to respect.
    def traced(a: int, b: int) -> int:
        c = a + b
        return c * 2

    traced.__jit__()
    assert traced(3, 4) == 14

    events = []

    def tracer(frame, event, arg):
        if frame.f_code.co_name == "traced":
            events.append(event)
        return tracer

    sys.settrace(tracer)
    try:
        assert traced(3, 4) == 14
    finally:
        sys.settrace(None)
    assert events[:1] == ["call"], events
    assert events[-1:] == ["return"], events

    monitored = []
    mon = sys.monitoring
    mon.use_tool_id(mon.PROFILER_ID, "jit snippet")
    try:
        mon.register_callback(
            mon.PROFILER_ID,
            mon.events.PY_START,
            lambda code, offset: monitored.append(code.co_name),
        )
        mon.set_events(mon.PROFILER_ID, mon.events.PY_START)
        try:
            assert traced(3, 4) == 14
        finally:
            mon.set_events(mon.PROFILER_ID, 0)
    finally:
        mon.free_tool_id(mon.PROFILER_ID)
    assert "traced" in monitored, monitored
