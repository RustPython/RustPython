from _js import Promise
from collections.abc import Coroutine, Awaitable
from abc import ABC, abstractmethod


def is_promise(prom):
    return callable(getattr(prom, "then", None))


def run(coro):
    """
    Run a coroutine. The coroutine should yield promise objects with a
    ``.then(on_success, on_error)`` method.
    """
    _Runner(coro)


class _Runner:
    def __init__(self, coro):
        self._send = coro.send
        self._throw = coro.throw
        # start the coro
        self.success(None)

    def _run(self, send, arg):
        try:
            ret = send(arg)
        except StopIteration:
            return
        ret.then(self.success, self.error)

    def success(self, res):
        self._run(self._send, res)

    def error(self, err):
        self._run(self._throw, err)


def main(async_func):
    """
    A decorator to mark a function as main. This calls run() on the
    result of the function, and logs an error that occurs.
    """
    run(_main_wrapper(async_func()))
    return async_func


async def _main_wrapper(coro):
    try:
        await coro
    except BaseException as e:
        for line in _format_exc(e, 1):
            print(line)


# TODO: get traceback/linecache working in wasm


def _format_exc(exc, skip_tb=0):
    exc_type, exc_value, exc_traceback = type(exc), exc, exc.__traceback__

    _str = _some_str(exc_value)

    yield "Traceback (most recent call last):"
    tb = exc_traceback
    while tb:
        if skip_tb:
            skip_tb -= 1
        else:
            co = tb.tb_frame.f_code
            yield f'  File "{co.co_filename}", line {tb.tb_lineno}, in {co.co_name}'
        tb = tb.tb_next

    stype = exc_type.__qualname__
    smod = exc_type.__module__
    if smod not in ("__main__", "builtins"):
        stype = smod + "." + stype

    yield _format_final_exc_line(stype, _str)


def _format_final_exc_line(etype, value):
    valuestr = _some_str(value)
    if value is None or not valuestr:
        line = "%s" % etype
    else:
        line = "%s: %s" % (etype, valuestr)
    return line


def _some_str(value):
    try:
        return str(value)
    except:
        return "<unprintable %s object>" % type(value).__name__


def _resolve(prom):
    if is_promise(prom):
        pass
    elif isinstance(prom, Coroutine):
        prom = _CoroPromise(prom)

    return Promise.resolve(prom)


class _CallbackMap:
    def __init__(self):
        self.done = 0
        self._successes = []
        self._errors = []

    def then(self, success=None, error=None):
        if success and not callable(success):
            raise TypeError("success callback must be callable")
        if error and not callable(error):
            raise TypeError("error callback must be callable")

        if self.done == -1:
            if error:
                return _call_resolve(error, self._error)
            else:
                return self
        elif self.done == 1:
            if success:
                return _call_resolve(success, self._result)
            else:
                return self

        if success:
            # def onsuccess(then=
            self._successes.append(success)
        if error:
            self._errors.append(error)

    def resolve(self, value):
        self._result = value
        self.done = 1
        for f in self._successes:
            f(value)
        del self._successes, self._errors

    def reject(self, err):
        self._result = err
        self.done = -1
        for f in self._errors:
            f(err)
        del self._successes, self._errors


class _CoroPromise:
    def __init__(self, coro):
        self._cbs = _CallbackMap()

        async def run_coro():
            try:
                res = await coro
            except BaseException as e:
                self._cbs.reject(e)
            else:
                self._cbs.resolve(res)

        run(run_coro())

    def then(self, on_success=None, on_failure=None):
        self._cbs.then(on_success, on_failure)


def _call_resolve(f, arg):
    try:
        ret = f(arg)
    except BaseException as e:
        return Promise.reject(e)
    else:
        return _resolve(ret)


def wait_all(proms):
    return Promise.resolve(_WaitAll(proms))


# basically an implementation of Promise.all
class _WaitAll:
    def __init__(self, proms):
        if not isinstance(proms, (list, tuple)):
            proms = tuple(proms)
        self._completed = 0
        self.cbs = _CallbackMap()
        num_proms = len(proms)
        self._results = [None] * num_proms

        # needs to be a separate function for creating a closure in a loop
        def register_promise(i, prom):
            completed = False

            def promise_done(success, res):
                nonlocal completed
                if completed or self.cbs.done:
                    return
                completed = True
                if success:
                    self._results[i] = res
                    self._completed += 1
                    if self._completed == num_proms:
                        results = tuple(self._results)
                        del self._results
                        self.cbs.resolve(results)
                else:
                    del self._results
                    self.cbs.reject(res)

            _resolve(prom).then(
                lambda res: promise_done(True, res),
                lambda err: promise_done(False, err),
            )

        for i, prom in enumerate(proms):
            register_promise(i, prom)

    def then(self, success=None, error=None):
        self.cbs.then(success, error)
