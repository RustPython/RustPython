import browser
import functools


# just setting up the framework, skip to the bottom to see the real code

ready = object()
go = object()


def run(coro, *, payload=None, error=False):
    send = coro.throw if error else coro.send
    try:
        cmd = send(payload)
    except StopIteration:
        return
    if cmd is ready:
        coro.send(
            (
                lambda *args: run(coro, payload=args),
                lambda *args: run(coro, payload=args, error=True),
            )
        )
    elif cmd is go:
        pass
    else:
        raise RuntimeError(f"expected cmd to be ready or go, got {cmd}")


class JSFuture:
    def __init__(self, prom):
        self._prom = prom

    def __await__(self):
        done, error = yield ready
        self._prom.then(done, error)
        res, = yield go
        return res


def wrap_prom_func(func):
    @functools.wraps(func)
    async def wrapper(*args, **kwargs):
        return await JSFuture(func(*args, **kwargs))

    return wrapper


fetch = wrap_prom_func(browser.fetch)

###################
# Real code start #
###################


async def main(delay):
    url = f"https://httpbin.org/delay/{delay}"
    print(f"fetching {url}...")
    res = await fetch(
        url, response_format="json", headers={"X-Header-Thing": "rustpython is neat!"}
    )
    print(f"got res from {res['url']}:")
    print(res, end="\n\n")


for delay in range(3):
    run(main(delay))
print()
