# Adapted from micropython-lib

# The MIT License (MIT)
#
# Copyright (c) 2013, 2014 micropython-lib contributors
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
# THE SOFTWARE.

import time
import logging
import types


log = logging.getLogger("asyncio")


# Workaround for not being able to subclass builtin types
class LoopStop(Exception):
    pass


class InvalidStateError(Exception):
    pass


# Object not matching any other object
_sentinel = []


class EventLoop:
    def __init__(self):
        self.q = []

    def call_soon(self, c, *args):
        self.q.append((c, args))

    def call_later(self, delay, c, *args):
        def _delayed(c, args, delay):
            yield from sleep(delay)
            self.call_soon(c, *args)

        Task(_delayed(c, args, delay))

    def run_forever(self):
        while self.q:
            f, args = self.q.pop(0)
            try:
                f(*args)
            except LoopStop:
                return
        # I mean, forever
        while True:
            time.sleep(1)

    def stop(self):
        def _cb():
            raise LoopStop

        self.call_soon(_cb)

    def run_until_complete(self, coro):
        t = ensure_future(coro)
        t.add_done_callback(lambda a: self.stop())
        self.run_forever()

    def close(self):
        pass


_def_event_loop = EventLoop()


class Future:
    def __init__(self, loop=_def_event_loop):
        self.loop = loop
        self.res = _sentinel
        self.cbs = []

    def result(self):
        if self.res is _sentinel:
            raise InvalidStateError
        return self.res

    def add_done_callback(self, fn):
        if self.res is _sentinel:
            self.cbs.append(fn)
        else:
            self.loop.call_soon(fn, self)

    def set_result(self, val):
        self.res = val
        for f in self.cbs:
            f(self)


class Task(Future):
    def __init__(self, coro, loop=_def_event_loop):
        super().__init__()
        self.loop = loop
        self.c = coro
        # upstream asyncio forces task to be scheduled on instantiation
        self.loop.call_soon(self)

    def __call__(self):
        try:
            next(self.c)
        except StopIteration as e:
            log.debug("Coro finished: %s", self.c)
            self.set_result(None)
        else:
            self.loop.call_soon(self)


def get_event_loop():
    return _def_event_loop


# Decorator
def coroutine(f):
    return f


def ensure_future(coro):
    if isinstance(coro, Future):
        return coro
    elif hasattr(coro, "__await__"):
        return ensure_future(_wrap_awaitable(coro))
    return Task(coro)


def _wrap_awaitable(awaitable):
    """Helper for asyncio.ensure_future().
    Wraps awaitable (an object with __await__) into a coroutine
    that will later be wrapped in a Task by ensure_future().
    """
    return (yield from awaitable.__await__())


class _Wait(Future):
    def __init__(self, n):
        super().__init__()
        self.n = n

    def _done(self):
        self.n -= 1
        log.debug("Wait: remaining tasks: %d", self.n)
        if not self.n:
            self.set_result(None)

    def __call__(self):
        pass


def wait(coro_list, loop=_def_event_loop):

    w = _Wait(len(coro_list))

    for c in coro_list:
        t = ensure_future(c)
        t.add_done_callback(lambda val: w._done())

    return w


@types.coroutine
def sleep(secs):
    t = time.time()
    log.debug("Started sleep at: %s, targetting: %s", t, t + secs)
    while time.time() < t + secs:
        time.sleep(0.01)
        yield
    log.debug("Finished sleeping %ss", secs)
