"""Stress contextvars from several threads at once.

A Context holds the variable map, and both the map and the per-variable cache
are shared between every thread that touches the Context. Reading and writing
them has to be done under a lock rather than a cell borrow.

Dropping a value that a set() or reset() displaced can run a __del__ that comes
straight back into the same Context, so the displaced value has to be released
after the lock is, not while it is held.
"""

import contextvars
import threading

ROUNDS = 2000

var = contextvars.ContextVar("v", default=0)
shared = contextvars.Context()
errors = []


class Reentrant:
    """__del__ runs while the variable that held this value is being replaced."""

    def __del__(self):
        try:
            var.get()
        except Exception:  # a different context, or no value: not what is tested
            pass


def churn():
    try:
        for i in range(ROUNDS):
            token = var.set(Reentrant())
            var.get()
            var.reset(token)
            var.set(i)
            var.get()
            contextvars.copy_context()
    except Exception as exc:  # noqa: BLE001
        errors.append(exc)


def run_in_shared():
    for i in range(ROUNDS):
        try:
            shared.run(var.set, i)
        except RuntimeError:
            # the Context is already entered by another thread
            pass


threads = [threading.Thread(target=churn) for _ in range(4)]
threads += [threading.Thread(target=run_in_shared) for _ in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()

assert not errors, errors

# the map itself still behaves
ctx = contextvars.copy_context()
ctx.run(var.set, 42)
assert ctx[var] == 42
assert var in ctx
assert list(ctx) == [var]
assert ctx.get(var) == 42
assert len(ctx) == 1

print("ok")
