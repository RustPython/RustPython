"""Advance the iterators of one tee() from several threads at once.

Every tee iterator reads its position, asks the shared buffer for that item and
then moves the position on. Reading and moving it on has to be one step, and
the buffer has to stay claimed until the value it fetched from the source is
cached: otherwise two callers work on the same index, a fetched value is
dropped, and the buffer is left to be filled out of order.

A caller that loses the race gets a RuntimeError, never a value another caller
has already been handed.
"""

import itertools
import threading

ROUNDS = 200
WORKERS = 4

errors = []


def drain(iterator, out):
    for _ in range(ROUNDS):
        try:
            out.append(next(iterator))
        except StopIteration:
            break
        except RuntimeError:
            # another thread is advancing this tee
            pass
        except Exception as exc:  # noqa: BLE001
            errors.append(exc)
            break


for _ in range(10):
    first, second = itertools.tee(iter(range(ROUNDS * WORKERS)))
    taken = [[] for _ in range(WORKERS)]
    threads = [
        threading.Thread(target=drain, args=(first if i % 2 else second, taken[i]))
        for i in range(WORKERS)
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    assert not errors, errors
    for got in taken:
        # one iterator hands out ascending values, each of them once
        assert got == sorted(set(got)), got
    for side in (taken[1], taken[3]), (taken[0], taken[2]):
        # the two threads sharing an iterator split its values between them
        shared = side[0] + side[1]
        assert len(shared) == len(set(shared)), shared

print("ok")
