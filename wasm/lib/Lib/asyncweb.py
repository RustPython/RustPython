from _js import Promise
from collections.abc import Coroutine

try:
    import browser
except ImportError:
    browser = None


def is_promise(prom):
    return callable(getattr(prom, "then", None))


def run(coro):
    """
    Run a coroutine. The coroutine should yield promise objects with a
    ``.then(on_success, on_error)`` method.
    """
    _Runner(coro)


def spawn(coro):
    """
    Run a coroutine. Like run(), but returns a promise that resolves with
    the result of the coroutine.
    """
    return _coro_promise(coro)


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
    except:  # noqa: E722
        import traceback
        import sys

        # TODO: sys.stderr on wasm
        traceback.print_exc(file=sys.stdout)


def _resolve(prom):
    if is_promise(prom):
        return prom
    elif isinstance(prom, Coroutine):
        return _coro_promise(prom)
    else:
        return Promise.resolve(prom)


class CallbackPromise:
    def __init__(self):
        self.done = 0
        self.__successes = []
        self.__errors = []

    def then(self, success=None, error=None):
        if success and not callable(success):
            raise TypeError("success callback must be callable")
        if error and not callable(error):
            raise TypeError("error callback must be callable")

        if not self.done:
            if success:
                self.__successes.append(success)
            if error:
                self.__errors.append(error)
            return

        cb = success if self.done == 1 else error
        if cb:
            return _call_resolve(cb, self.__result)
        else:
            return self

    def __await__(self):
        yield self

    def resolve(self, value):
        if self.done:
            return
        self.__result = value
        self.done = 1
        for f in self.__successes:
            f(value)
        del self.__successes, self.__errors

    def reject(self, err):
        if self.done:
            return
        self.__result = err
        self.done = -1
        for f in self.__errors:
            f(err)
        del self.__successes, self.__errors


def _coro_promise(coro):
    prom = CallbackPromise()

    async def run_coro():
        try:
            res = await coro
        except BaseException as e:
            prom.reject(e)
        else:
            prom.resolve(res)

    run(run_coro())

    return prom


def _call_resolve(f, arg):
    try:
        ret = f(arg)
    except BaseException as e:
        return Promise.reject(e)
    else:
        return _resolve(ret)


# basically an implementation of Promise.all
def wait_all(proms):
    cbs = CallbackPromise()

    if not isinstance(proms, (list, tuple)):
        proms = tuple(proms)
    num_completed = 0
    num_proms = len(proms)

    if num_proms == 0:
        cbs.resolve(())
        return cbs

    results = [None] * num_proms

    # needs to be a separate function for creating a closure in a loop
    def register_promise(i, prom):
        prom_completed = False

        def promise_done(success, res):
            nonlocal prom_completed, results, num_completed
            if prom_completed or cbs.done:
                return
            prom_completed = True
            if success:
                results[i] = res
                num_completed += 1
                if num_completed == num_proms:
                    result = tuple(results)
                    del results
                    cbs.resolve(result)
            else:
                del results
                cbs.reject(res)

        _resolve(prom).then(
            lambda res: promise_done(True, res),
            lambda err: promise_done(False, err),
        )

    for i, prom in enumerate(proms):
        register_promise(i, prom)

    return cbs


if browser:
    _settimeout = browser.window.get_prop("setTimeout")

    def timeout(ms):
        prom = CallbackPromise()

        @browser.jsclosure_once
        def cb(this):
            print("AAA")
            prom.resolve(None)

        _settimeout.call(cb.detach(), browser.jsfloat(ms))
        return prom
