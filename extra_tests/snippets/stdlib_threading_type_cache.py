"""Stress the lock-free type method cache against concurrent type mutation.

Readers hammer method lookups while a mutator continuously replaces and
deletes the method, dropping the old function objects. Guards against
use-after-free in the cache read protocol (QSBR deferred reclamation).
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

def mutator(stop):
    i = 0
    while not stop.is_set():
        def m(self, _i=i):
            return _i
        C.m = m
        i += 1
        if i % 97 == 0:
            try:
                del C.m
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
