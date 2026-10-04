"""Stress set repr against concurrent mutation.

repr() checks that the set is non-empty and then reads its first element. The
two steps are separate, so another thread can empty the set in between; the
read has to cope with that rather than trusting the earlier check.

Threads that observe a mutation mid-iteration raise RuntimeError, which is a
legitimate outcome here; a regression shows up as a crash instead.
"""

import threading

shared_set = {1, 2, 3, 4, 5}
stop = False


def mutate():
    while not stop:
        try:
            shared_set.clear()
            shared_set.update({1, 2, 3})
        except RuntimeError:  # changed size during iteration
            pass


def read():
    for _ in range(20000):
        try:
            repr(shared_set)
        except RuntimeError:  # changed size during iteration
            pass


mutators = [threading.Thread(target=mutate) for _ in range(2)]
readers = [threading.Thread(target=read) for _ in range(2)]
for t in mutators + readers:
    t.start()
for t in readers:
    t.join()
stop = True
for t in mutators:
    t.join()

print("ok")
