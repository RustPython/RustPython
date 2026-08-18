"""Stress itertools.cycle from several threads at once.

cycle() advances its index and wraps it back to zero when it reaches the end of
the saved items. Doing that in two separate steps lets another thread observe
the index past the end and read out of bounds, so the update has to be a single
atomic step.
"""

import itertools
import threading

shared_cycle = itertools.cycle([1, 2, 3])


def spin():
    for _ in range(20000):
        next(shared_cycle)


threads = [threading.Thread(target=spin) for _ in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()

print("ok")
