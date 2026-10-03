"""Run unchanged on CPython 3.15rc2 and RustPython; compare JSON output.

No sockets or subprocesses are used.  A tiny scheduling loop isolates the
initialization contract from this executor's AF_UNIX restriction.
"""

import asyncio
import json
import sys
from asyncio import futures, tasks


class Loop:
    def __init__(self):
        self.ready = []

    def get_debug(self):
        return False

    def is_running(self):
        return False

    def call_soon(self, callback, *args, context=None):
        self.ready.append((callback, args, context))

    def call_exception_handler(self, context):
        raise AssertionError(context)

    def step(self):
        callback, args, context = self.ready.pop(0)
        previous = asyncio.events._get_running_loop()
        asyncio.events._set_running_loop(self)
        try:
            if context is None:
                callback(*args)
            else:
                context.run(callback, *args)
        finally:
            asyncio.events._set_running_loop(previous)


def rejected(callback, cls, native):
    try:
        callback()
    except RuntimeError as exc:
        if native:
            name = cls.__qualname__
            if cls.__module__ not in ("builtins", "__main__"):
                name = cls.__module__ + "." + name
        else:
            name = cls.__name__
        assert str(exc) == name + " object is already initialized", str(exc)
    else:
        raise AssertionError("reinitialization was accepted")


observed = []
future_classes = [("native", futures._CFuture), ("python", futures._PyFuture)]
if "--tasks-only" in sys.argv:
    future_classes = []
for label, base in future_classes:

    class Derived(base):
        pass

    for cls in [base, Derived]:
        for state in ["pending", "finished", "cancelled"]:
            loop = Loop()
            f = cls(loop=loop)
            token = object()
            if state == "finished":
                f.set_result(token)
            elif state == "cancelled":
                f.cancel()
            callback = lambda future: None
            f.add_done_callback(callback)
            before = (f.done(), f.cancelled(), f._callbacks, len(loop.ready))
            rejected(lambda: f.__init__(loop=Loop()), cls, label == "native")
            assert f.get_loop() is loop
            assert (f.done(), f.cancelled(), f._callbacks, len(loop.ready)) == before
            if state == "finished":
                assert f.result() is token
            observed.append([label, "Future", cls is not base, state, "preserved"])

for label, base in [("native", tasks._CTask), ("python", tasks._PyTask)]:

    class Derived(base):
        pass

    for cls in [base, Derived]:
        for state in ["pending", "finished", "cancelled"]:
            loop = Loop()
            token = object()

            async def original():
                return token

            async def replacement():
                raise AssertionError("replacement coroutine must never run")

            coro, other = original(), replacement()
            t = cls(coro, loop=loop, name="original")
            t._log_destroy_pending = False
            if state == "finished":
                loop.step()
            elif state == "cancelled":
                t.cancel()
                loop.step()
            before = (t.done(), t.cancelled(), t.cancelling(), len(loop.ready))
            try:
                rejected(
                    lambda: t.__init__(other, loop=Loop(), name="replacement"),
                    cls,
                    label == "native",
                )
                # Rejection must precede coroutine validation on an initialized task.
                rejected(
                    lambda: t.__init__(object(), loop=Loop()), cls, label == "native"
                )
                assert t.get_loop() is loop
                assert t.get_coro() is coro
                assert t.get_name() == "original"
                assert (
                    t.done(),
                    t.cancelled(),
                    t.cancelling(),
                    len(loop.ready),
                ) == before
                if state == "finished":
                    assert t.result() is token
                if state == "pending":
                    assert len(loop.ready) == 1
                    loop.step()
                    assert t.result() is token
            finally:
                other.close()
                coro.close()
            observed.append([label, "Task", cls is not base, state, "preserved"])

print(json.dumps(observed))
