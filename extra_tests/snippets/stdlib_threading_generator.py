"""Resume one generator from several threads at once.

A generator is resumed by one thread at a time, and whether the sent value is
pushed onto the frame's value stack depends on whether the generator has
already started. Deciding that before the generator is claimed reads a frame
another thread can advance in the meantime, and resuming it then leaves the
stack short of what the code after the yield expects.

Every yielded value still has to reach exactly one caller: threads that lose
the race get a ValueError instead of a value.
"""

import threading

WORKERS = 4
ROUNDS = 400


def counter():
    yield 1
    yield 2
    yield 3


gens = [counter() for _ in range(ROUNDS)]
received = [[] for _ in range(ROUNDS)]
start = threading.Barrier(WORKERS)
errors = []


def worker():
    try:
        for index, gen in enumerate(gens):
            start.wait()
            for _ in range(3):
                try:
                    received[index].append(next(gen))
                except StopIteration:
                    break
                except ValueError:
                    # another thread is running this generator
                    pass
    except Exception as exc:  # noqa: BLE001
        errors.append(exc)
        # the other workers are waiting at the barrier for this one
        start.abort()


threads = [threading.Thread(target=worker) for _ in range(WORKERS)]
for t in threads:
    t.start()
for t in threads:
    t.join()

assert not errors, errors
for got in received:
    # no value handed out twice, and none skipped
    assert sorted(got) == list(range(1, len(got) + 1)), got


# a generator that is closed while it is being resumed stays consistent
def loop():
    while True:
        yield 1


shared = loop()
closed = threading.Barrier(2)


def resumer():
    closed.wait()
    for _ in range(ROUNDS):
        try:
            next(shared)
        except (StopIteration, ValueError):
            pass


def closer():
    closed.wait()
    try:
        shared.close()
    except ValueError:
        # the generator was running
        pass


pair = [threading.Thread(target=resumer), threading.Thread(target=closer)]
for t in pair:
    t.start()
for t in pair:
    t.join()

print("ok")
