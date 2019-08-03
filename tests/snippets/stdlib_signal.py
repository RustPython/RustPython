import signal
import time
import sys
from testutils import assert_raises

assert_raises(TypeError, lambda: signal.signal(signal.SIGINT, 2))

signals = []

def handler(signum, frame):
	signals.append(signum)


signal.signal(signal.SIGILL, signal.SIG_IGN);
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



