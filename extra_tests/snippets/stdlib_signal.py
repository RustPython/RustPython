import queue
import signal
import sys
import threading
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

    q = queue.SimpleQueue()
    handled_at = []

    def queue_handler(signum, frame):
        handled_at.append(time.monotonic() - start)

    signal.signal(signal.SIGALRM, queue_handler)
    start = time.monotonic()
    signal.setitimer(signal.ITIMER_REAL, 0.1)

    def unblock():
        time.sleep(1)
        q.put("unblock")

    thread = threading.Thread(target=unblock)
    thread.start()
    try:
        assert q.get() == "unblock"
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        thread.join()

    # This leaves a 0.3-second margin before the thread unblocks at one second.
    assert handled_at and handled_at[0] < 0.7
