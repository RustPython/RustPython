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


# unix
if "win" not in sys.platform:
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
