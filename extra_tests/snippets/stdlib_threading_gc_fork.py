"""Fork while other threads drive concurrent GC stop-the-world.

fork() and the cycle collector both stop the world through the same shared
state. Without a single exclusion around each stop->start span, an interleaving
of the fork requester and a GC requester clobbers that state (requester word,
suspension countdown) so the completion check never converges and a requester
waits on itself forever.

Worker threads allocate cyclic garbage with GC enabled while the main thread
forks repeatedly; each child collects and exits. A regression shows up as a
hang in the parent (never finishing the fork loop). The allocation rate is kept
light so the collection stays cheap even in unoptimized builds.
"""

import gc
import os
import threading
import time

if not hasattr(os, "fork"):
    print("skipped (no fork)")
    raise SystemExit(0)

gc.enable()
stop = threading.Event()


def churn():
    while not stop.is_set():
        a = {}
        b = {"a": a}
        a["b"] = b  # cycle collectable only by the cycle collector
        lst = [a, b]
        lst.append(lst)
        del a, b, lst
        # Throttle so the collector keeps the heap small; the point is to
        # interleave fork with concurrent collections, not to grow the heap.
        time.sleep(0.001)


workers = [threading.Thread(target=churn) for _ in range(4)]
for w in workers:
    w.start()

# Let the workers get going before forking.
time.sleep(0.05)

N = 25
for _ in range(N):
    pid = os.fork()
    if pid == 0:
        # Child: run its own stop-the-world collection, then exit.
        gc.collect()
        os._exit(0)
    _, status = os.waitpid(pid, 0)
    assert status == 0, status

stop.set()
for w in workers:
    w.join()

print("ok")


def test_fork_from_gc_finalizer():
    enabled = gc.isenabled()
    gc.disable()
    child_pid = None
    nested_collection = None

    class ForkingCycle:
        def __del__(self):
            nonlocal child_pid, nested_collection
            child_pid = os.fork()
            if child_pid == 0:
                cycle = []
                cycle.append(cycle)
                del cycle
                # The outer collector's guard survives fork and still excludes
                # another collection until this finalizer returns.
                nested_collection = gc.collect()

    try:
        cycle = ForkingCycle()
        cycle.link = cycle
        del cycle
        gc.collect()
        if child_pid == 0:
            collected_after = gc.collect()
            os._exit(0 if nested_collection == 0 and collected_after > 0 else 1)
        assert child_pid is not None and child_pid > 0
        _, status = os.waitpid(child_pid, 0)
        assert status == 0, status
    finally:
        if enabled:
            gc.enable()


test_fork_from_gc_finalizer()
