"""A rejected native Task reinitialization must preserve the pending task."""

import asyncio
from asyncio import tasks


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


def rejected(callback):
    try:
        callback()
    except RuntimeError:
        pass
    else:
        raise AssertionError("reinitialization was accepted")


def test_task_reinitialization_preserves_pending_task():
    loop = Loop()
    token = object()

    async def original():
        return token

    async def replacement():
        raise AssertionError("replacement coroutine must never run")

    coro, other = original(), replacement()
    t = tasks._CTask(coro, loop=loop, name="original")
    t._log_destroy_pending = False
    before = (t.done(), t.cancelled(), t.cancelling(), len(loop.ready))
    try:
        rejected(lambda: t.__init__(other, loop=Loop(), name="replacement"))
        # Rejection must precede coroutine validation on an initialized task.
        rejected(lambda: t.__init__(object(), loop=Loop()))
        assert t.get_loop() is loop
        assert t.get_coro() is coro
        assert t.get_name() == "original"
        assert (t.done(), t.cancelled(), t.cancelling(), len(loop.ready)) == before
        assert len(loop.ready) == 1
        loop.step()
        assert t.result() is token
    finally:
        other.close()
        coro.close()


test_task_reinitialization_preserves_pending_task()
