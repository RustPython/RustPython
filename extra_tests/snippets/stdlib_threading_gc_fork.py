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
import subprocess
import sys
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


# The surviving native stack can hold the collector or type-mutation guard.
# Child repair must preserve it until that original stack releases it.
for source in (
    """
gc.disable()
pid = -1
def callback(phase, info):
    global pid
    if phase == 'start' and pid == -1:
        pid = os.fork()
gc.callbacks.append(callback)
gc.collect()
gc.callbacks.remove(callback)
""",
    """
pid = -1
class Meta(type):
    def mro(cls):
        global pid
        if pid == -1:
            pid = os.fork()
        return super().mro()
class Sample(metaclass=Meta):
    pass
Sample.value = 42
assert Sample.value == 42
""",
):
    subprocess.run(
        [
            sys.executable,
            "-c",
            "import gc, os\n"
            + source
            + """
for _ in range(3):
    node = []
    node.append(node)
    del node
    gc.collect()
if pid == 0:
    os._exit(0)
assert os.waitpid(pid, 0)[1] == 0
""",
        ],
        check=True,
        timeout=30,
    )
