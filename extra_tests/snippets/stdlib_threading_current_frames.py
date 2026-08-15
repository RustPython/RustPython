"""Take sys._current_frames() while other threads are running Python.

The frame each thread is executing is published for cross-thread readers, and
_current_frames() takes a reference to it with the world stopped. A reader that
disagrees with the publisher about what the published pointer addresses reads
and reference-counts the wrong memory, which corrupts a neighbouring object
rather than failing at the read: the damage surfaces later, in the thread that
owns it, as a crash or a wedge.

Workers therefore run ordinary Python calls (which publish a frame) in a tight
loop while the main thread hammers _current_frames().
"""

import sys
import threading
import time

DURATION = 1.5


def leaf():
    return sum(range(8))


def nest(n):
    if n:
        return nest(n - 1)
    return leaf()


def worker(stop):
    while not stop.is_set():
        nest(16)


def frames_are_sane(frames):
    # Every key is a thread id, every value a frame of this process.
    for tid, frame in frames.items():
        assert isinstance(tid, int), tid
        assert tid > 0, tid
        assert type(frame).__name__ == "frame", frame
        assert isinstance(frame.f_lineno, int), frame
        assert isinstance(frame.f_code.co_name, str), frame


# The main thread sees itself where it stands.
me = sys._current_frames()[threading.get_ident()]
assert me is sys._getframe(), me

stop = threading.Event()
threads = [threading.Thread(target=worker, args=(stop,)) for _ in range(4)]
for t in threads:
    t.start()

deadline = time.time() + DURATION
calls = 0
while time.time() < deadline:
    frames_are_sane(sys._current_frames())
    calls += 1
stop.set()
for t in threads:
    t.join()

assert calls > 0, calls


# A thread parked in a call the main thread can name is reported inside it,
# with its callers reachable through f_back.
entered = threading.Event()
leave = threading.Event()
seen = []


def g456():
    seen.append(threading.get_ident())
    entered.set()
    leave.wait()


def f123():
    g456()


t = threading.Thread(target=f123)
t.start()
entered.wait()
try:
    chain = []
    frame = sys._current_frames()[seen[0]]
    while frame is not None:
        chain.append(frame.f_code.co_name)
        frame = frame.f_back
    assert "g456" in chain, chain
    assert chain.index("g456") < chain.index("f123"), chain
finally:
    leave.set()
    t.join()

print("ok")
