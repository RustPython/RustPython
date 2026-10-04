"""A blocking queue.SimpleQueue.get() must stay responsive to signals.

SimpleQueue.get() waits on a Condvar. A single uninterrupted wait blocks
Python-level signal handlers (including the default KeyboardInterrupt) until
the wait itself returns, since signals are only delivered at bytecode
safepoints. A regression shows up as the handler firing only once get()
unblocks, instead of promptly when the signal actually arrives.
"""

import queue
import signal
import sys
import threading
import time

if sys.platform.startswith("win"):
    print("skipped (no SIGALRM)")
    raise SystemExit(0)

q = queue.SimpleQueue()
start = time.time()
handled_at = []


def handler(signum, frame):
    handled_at.append(time.time() - start)


signal.signal(signal.SIGALRM, handler)
signal.setitimer(signal.ITIMER_REAL, 0.3)


def unblock_later():
    time.sleep(1.5)
    q.put("unblock")


threading.Thread(target=unblock_later, daemon=True).start()

item = q.get()  # blocks until unblock_later() wakes us up
elapsed = time.time() - start

assert item == "unblock", item
assert handled_at, "signal handler never ran"
# The handler should fire around t=0.3s, when the timer was started, not t=1.5s
# (when get() finally unblocked).
assert handled_at[0] < elapsed - 0.5, (
    f"signal handled at {handled_at[0]:.2f}s but get() only returned at "
    f"{elapsed:.2f}s -- signal was not processed while blocked"
)

print("ok")
