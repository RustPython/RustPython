import signal
import sys
import time

from testutils import assert_raises

assert_raises(TypeError, lambda: signal.signal(signal.SIGINT, 2))

signals = []


def handler(signum, frame):
    signals.append(signum)


signal.signal(signal.SIGILL, signal.SIG_IGN)
assert signal.getsignal(signal.SIGILL) is signal.SIG_IGN

old_signal = signal.signal(signal.SIGILL, signal.SIG_DFL)
assert old_signal is signal.SIG_IGN
assert signal.getsignal(signal.SIGILL) is signal.SIG_DFL


# Ignoring a synchronously raised signal must not invoke a callback, including
# on WASI where libc uses a function pointer rather than integer 1 for SIG_IGN.
synchronous_signals = []


def synchronous_handler(signum, frame):
    synchronous_signals.append(signum)


previous = signal.signal(signal.SIGILL, synchronous_handler)
try:
    signal.raise_signal(signal.SIGILL)
    assert synchronous_signals == [signal.SIGILL]
    assert signal.signal(signal.SIGILL, signal.SIG_IGN) is synchronous_handler
    signal.raise_signal(signal.SIGILL)
    assert synchronous_signals == [signal.SIGILL]
    assert signal.getsignal(signal.SIGILL) is signal.SIG_IGN
finally:
    signal.signal(signal.SIGILL, previous)

if sys.platform == "wasi":
    # wasi-libc rejects SIGKILL (9) and SIGSTOP (19), even though they are not
    # exported by RustPython's minimal WASI signal module.
    for uncatchable in (9, 19):
        previous = signal.getsignal(uncatchable)
        assert_raises(OSError, signal.signal, uncatchable, signal.SIG_IGN)
        assert signal.getsignal(uncatchable) is previous


# unix, and not WASI
if "win" not in sys.platform and sys.platform != "wasi":
    signal.signal(signal.SIGALRM, handler)
    assert signal.getsignal(signal.SIGALRM) is handler

    signal.alarm(1)
    time.sleep(2.0)
    assert signals == [signal.SIGALRM]

    signal.signal(signal.SIGALRM, signal.SIG_IGN)
    signal.alarm(1)
    time.sleep(2.0)

    assert signals == [signal.SIGALRM]

    signal.signal(signal.SIGALRM, handler)
    signal.alarm(1)
    time.sleep(2.0)

    assert signals == [signal.SIGALRM, signal.SIGALRM]

    # A handler may call signal.signal(), and the usual reason is to disarm
    # itself. Reading the handler table while the handler runs used to be a
    # crash rather than a rearm.
    rearmed = []

    def rearm(signum, frame):
        rearmed.append(signum)
        signal.signal(signal.SIGALRM, signal.SIG_IGN)

    signal.signal(signal.SIGALRM, rearm)
    signal.raise_signal(signal.SIGALRM)
    assert rearmed == [signal.SIGALRM], rearmed
    assert signal.getsignal(signal.SIGALRM) is signal.SIG_IGN

    # The same goes for arming a different signal from inside a handler.
    armed = []

    def target(signum, frame):
        armed.append("target")

    def arm_other(signum, frame):
        armed.append("arm_other")
        signal.signal(signal.SIGUSR2, target)

    signal.signal(signal.SIGUSR1, arm_other)
    signal.raise_signal(signal.SIGUSR1)
    assert armed == ["arm_other"], armed
    assert signal.getsignal(signal.SIGUSR2) is target

    signal.raise_signal(signal.SIGUSR2)
    assert armed == ["arm_other", "target"], armed

    signal.signal(signal.SIGALRM, signal.SIG_DFL)
    signal.signal(signal.SIGUSR1, signal.SIG_DFL)
    signal.signal(signal.SIGUSR2, signal.SIG_DFL)
