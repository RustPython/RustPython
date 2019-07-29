import signal
import time

signals = []

def handler(signum, frame):
	signals.append(signum)


signal.signal(14, handler)
assert signal.getsignal(14) is handler

signal.alarm(2)
time.sleep(3.0)
assert signals == [14]

