"""Stress GC traversal against concurrently executing frames.

The cycle collector reads each tracked object's interpreter state, including
the data stack and fast locals of frames that other threads are actively
executing. Those slots are written without synchronization by the running
thread, so the collector must only read them while the world is stopped.

Workers churn frame state hard: deep recursion (many nested frames), heavy
local rebind / stack traffic, and generators repeatedly resumed. Meanwhile a
collector thread loops gc.collect() and an introspector walks live frame
objects via gc.get_objects(). A regression (torn read of a running frame)
shows up as a crash, a use-after-free, or a hang.
"""

import gc
import sys
import threading
import time

DURATION = 1.5


def deep(n):
    # Deep recursion + local rebind churns fast locals and the data stack.
    a = n
    b = [n, n + 1]
    c = {"k": a}
    if n <= 0:
        return a + len(b) + len(c)
    a = a - 1
    b.append(a)
    return deep(n - 1) + a


def gen_worker():
    def counter(limit):
        acc = 0
        i = 0
        while i < limit:
            box = {"i": i}
            box["self"] = box  # a cycle held by the running generator frame
            acc += i
            yield acc
            i += 1

    g = counter(200)
    total = 0
    for v in g:
        total += v
    return total


def make_frame_cycles(n):
    for _ in range(n):

        def inner():
            fr = sys._getframe()
            box = {"fr": fr}
            box["self"] = box
            return None

        inner()


def worker(stop):
    # deep() nesting is kept modest so the recursion also fits the smaller
    # worker-thread stack of unoptimized (debug) builds; the generators and
    # frame cycles supply the rest of the frame churn.
    while not stop.is_set():
        deep(12)
        gen_worker()
        make_frame_cycles(20)


def collector(stop):
    while not stop.is_set():
        gc.collect()


def introspector(stop):
    while not stop.is_set():
        for o in gc.get_objects():
            if type(o).__name__ == "frame":
                try:
                    _ = o.f_lineno
                    _ = o.f_code.co_name
                except Exception:
                    pass


stop = threading.Event()
threads = [threading.Thread(target=worker, args=(stop,)) for _ in range(4)]
threads.append(threading.Thread(target=collector, args=(stop,)))
threads.append(threading.Thread(target=introspector, args=(stop,)))
for t in threads:
    t.start()
time.sleep(DURATION)
stop.set()
for t in threads:
    t.join()
print("ok")
