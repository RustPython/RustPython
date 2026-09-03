# Automatic compilation must be invisible: every assertion below has to hold
# whether or not functions were compiled behind our back.
import sys

# `_stats` is a RustPython addition, so this stays False under CPython even on a
# build whose own JIT is enabled.
AOT = sys._jit.is_enabled() and hasattr(sys._jit, "_stats")

# The automatic path waits for a function to be called often enough to be worth
# compiling, and takes the types it specializes on from the call that crosses
# that line. So a function has to be warmed with the arguments the assertion
# below it means to test, or the assertion runs against the interpreter and
# proves nothing about compiled code. Comfortably more than the threshold: if
# it ever rises past this, the stat floors at the bottom say so.
WARMUP = 200


def warm(f, *args):
    for _ in range(WARMUP):
        f(*args)
    return f


def scale(a: float, b: float) -> float:
    return a * b + a - b


def wide(a: int, b: int) -> int:
    return a + b


def untyped(a, b):
    return a + b


def variadic(*args) -> int:
    total = 0
    for a in args:
        total = total + a
    return total


def closure_factory(n: int):
    def inner(x: int) -> int:
        return x + n

    return inner


def generator(n: int):
    yield n


def guarded(a: int) -> int:
    try:
        return a
    except ValueError:
        return 0


warm(scale, 2.0, 3.0)
assert scale(2.0, 3.0) == 5.0

# An integer where the warm-up saw a float does not fit the compiled signature.
# The call has to fall back rather than reinterpret the bits.
assert scale(2, 3.0) == 5.0
assert scale(2.0, 3.0) == 5.0

# Integer arithmetic widens once the machine word can no longer hold the
# answer. `wide` compiles under the automatic path too now, and deoptimizes
# back to the interpreter exactly where that widening has to happen.
warm(wide, 3, 4)
assert wide(3, 4) == 7
assert wide(2**62, 2**62) == 2**63


# The guard above deoptimizes `wide`, and a deopt discards the compiled
# code and leaves the function permanently interpreted - so a third
# widening case on `wide` itself would never touch compiled code again.
# A fresh function gets its own compile attempt for it.
def wide2(a: int, b: int) -> int:
    return a + b


warm(wide2, 3, 4)
assert wide2(-(2**63), -(2**63)) == -(2**64)

# Shapes with no compiled form at all still behave, warm or cold.
assert untyped("a", "b") == "ab"
assert untyped(1, 2) == 3
assert warm(variadic, 1, 2, 3)(1, 2, 3) == 6
assert warm(closure_factory(5), 1)(1) == 6
assert list(generator(7)) == [7]
assert warm(guarded, 3)(3) == 3

# A zero operand is ordinary for `scale`, which only multiplies and adds.
assert scale(1.0, 0.0) == 1.0


def divide(a: float, b: float) -> float:
    return a / b


warm(divide, 1.0, 2.0)
assert divide(1.0, 2.0) == 0.5
try:
    divide(1.0, 0.0)
except ZeroDivisionError:
    pass
else:
    raise AssertionError("expected ZeroDivisionError")


# A compiled self-call resolves the global by name, but the interpreter reads
# the globals dict on every call. Rebinding the name has to be observable.
def countdown(a: float) -> float:
    if a > 0.0:
        return countdown(a - 1.0)
    return a


warm(countdown, 3.0)
assert countdown(3.0) == 0.0
original_countdown = countdown


def countdown(a: float) -> float:
    return -1.0


warm(countdown, 3.0)
assert original_countdown(3.0) == -1.0


def fib_iter(n: int) -> int:
    a = 0
    b = 1
    i = 0
    while i < n:
        a, b = b, a + b
        i = i + 1
    return a


warm(fib_iter, 10)
assert fib_iter(10) == 55
# Overflowing partway through is where the compiled loop hands back to the
# interpreter, which finishes it as a bignum.
assert fib_iter(95) == 31940434634990099905


# Nothing the automatic path does runs Python code, annotations included.
# Under PEP 649 reading them means calling `__annotate__`, and a program that
# never asked for its own annotations must not have them evaluated behind its
# back - here that would raise, since the name is not defined yet.
def forward(a: NotDefinedYet) -> int:
    return 1


warm(forward, 1)
assert forward(1) == 1
try:
    forward.__annotations__
except NameError:
    pass
else:
    raise AssertionError("expected the forward reference to still be unresolved")


if AOT:
    # The same thing where evaluating the annotation would be unmissable.
    class Boom(BaseException):
        pass

    def explode():
        raise Boom

    def annotated(a: explode()) -> int:
        return 1

    warm(annotated, 1)
    assert annotated(1) == 1
    try:
        annotated.__annotations__
    except Boom:
        pass
    else:
        raise AssertionError("expected the annotation to still be unevaluated")


# A frame that outlives its call has to keep resolving `f_back` past its
# immediate caller. Each frame on the way back gets a frame object only
# because the one below it returned and asked for one, so a link that stops
# after the first hop hides the entire stack behind it.
def innermost():
    return sys._getframe()


def middle():
    return innermost()


def outermost():
    return middle()


walked = []
frame = outermost()
while frame is not None:
    walked.append(frame.f_code.co_name)
    frame = frame.f_back
assert walked[:4] == ["innermost", "middle", "outermost", "<module>"], walked


# A compiled loop has to be leaveable. Native code that polls for nothing can
# never park for a stop-the-world, so a thread inside one holds up every
# collection the rest of the process asks for - not for a while, but for good.
# There is nothing to assert: the collection below either returns or it does
# not.
import gc
import threading
import time


def spin(n: int) -> int:
    i = 0
    while i < n:
        i = i + 1
    return i


# Warm on a count that returns at once, so the thread below enters a loop that
# is already compiled - which is the whole point of the check.
warm(spin, 0)

spinning = threading.Event()


def keep_spinning():
    spinning.set()
    # Further than this script will ever get. The thread is a daemon and is
    # meant to be left exactly where it is.
    spin(1 << 62)


threading.Thread(target=keep_spinning, daemon=True).start()
spinning.wait()
# Long enough that the thread is inside the loop rather than on its way in.
time.sleep(0.1)
gc.collect()


if AOT:
    compiled, rejected, deoptimized = sys._jit._stats()
    # `scale`, `wide`, `wide2`, `divide`, `fib_iter`, `spin`, `forward`,
    # `annotated`, `warm` itself and the rebound `countdown` are what the
    # automatic path takes above - the original self-recursive `countdown`,
    # kept as `original_countdown`, is refused. These are floors, not exact
    # counts, but they must not regress: a floor already met before a change
    # cannot tell whether the gate came back.
    assert compiled >= 7, (compiled, rejected, deoptimized)
    # ... the int argument in `scale(2, 3.0)` handed it back, and
    # `fib_iter(95)` overflows partway through its loop.
    assert deoptimized >= 5, (compiled, rejected, deoptimized)
    # Everything else was turned down rather than mis-compiled. `rejected`
    # stays a loose floor: it moves with the feature set of the binary.
    assert rejected >= 1, (compiled, rejected, deoptimized)
    print("aot: compiled", compiled, "rejected", rejected, "deopt", deoptimized)
