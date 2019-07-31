import signal
import time
from testutils import assert_raises

assert_raises(TypeError, lambda: signal.signal(signal.SIGINT, 2))

signals = []

def handler(signum, frame):
	signals.append(signum)


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



