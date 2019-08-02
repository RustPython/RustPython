import signal
import time
import sys
from testutils import assert_raises

signals = []

def handler(signum, frame):
	signals.append(signum)


# unix
if "win" not in sys.platform:
	assert_raises(TypeError, lambda: signal.signal(signal.SIGINT, 2))

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



