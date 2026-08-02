"""Stress the lock-free type method cache against concurrent type mutation.

Readers hammer method lookups while a mutator continuously replaces and
deletes the method, dropping the old function objects. Guards against
use-after-free in the cache read protocol (QSBR deferred reclamation).

Also churns a freelist-eligible published value (a tuple class attribute):
tuples normally go back through the freelist on dealloc, but once one is
published to the type cache it must instead go through the QSBR-deferred
reclamation path, so this exercises that bypass.
"""

import threading
import time


class C:
    def m(self):
        return -1


DURATION = 1.5


def reader(stop):
    obj = C()
    while not stop.is_set():
        for _ in range(1000):
            try:
                obj.m()
            except AttributeError:
                pass
            try:
                obj.shape
            except AttributeError:
                pass


def mutator(stop):
    i = 0
    while not stop.is_set():

        def m(self, _i=i):
            return _i

        C.m = m
        C.shape = (i, i + 1)
        i += 1
        if i % 97 == 0:
            try:
                del C.m
            except AttributeError:
                pass
            try:
                del C.shape
            except AttributeError:
                pass


stop = threading.Event()
threads = [threading.Thread(target=reader, args=(stop,)) for _ in range(4)]
threads.append(threading.Thread(target=mutator, args=(stop,)))
for t in threads:
    t.start()
time.sleep(DURATION)
stop.set()
for t in threads:
    t.join()
print("ok")
