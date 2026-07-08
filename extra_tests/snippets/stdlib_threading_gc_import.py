"""Concurrent imports plus GC stop-the-world must not deadlock (no fork).

The global import lock is held across bytecode by the importlib bootstrap, so
its holder can be parked at a safepoint mid-hold. If another thread blocks on
that lock while attached, a GC stop-the-world requester waits forever for that
attached thread to suspend while the lock holder stays parked -- a three-party
deadlock. Acquiring the import lock must therefore detach so the wait honors a
stop-the-world request.

Two threads repeatedly re-import modules (contending the import lock) while a
third storms the cycle collector and a fourth allocates cyclic garbage. A
regression shows up as a hang (the importer threads never finishing).
"""

import gc
import importlib
import sys
import threading

gc.enable()
stop = threading.Event()

# Modules cheap to import and safe to drop/re-import repeatedly.
MODS = ("colorsys", "stringprep")
ITERS = 2000


def importer(mod):
    for _ in range(ITERS):
        if stop.is_set():
            break
        sys.modules.pop(mod, None)
        importlib.import_module(mod)


def collector():
    while not stop.is_set():
        gc.collect()


def allocator():
    while not stop.is_set():
        y = [{"i": i} for i in range(50)]
        y[0]["self"] = y  # cycle collectable only by the cycle collector


importers = [threading.Thread(target=importer, args=(m,)) for m in MODS]
helpers = [threading.Thread(target=collector), threading.Thread(target=allocator)]

for t in importers + helpers:
    t.start()
for t in importers:
    t.join()
stop.set()
for t in helpers:
    t.join()

print("ok")
