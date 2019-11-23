import asyncio


class ContextManager:
    async def __aenter__(self):
        print("Entrada")
        ls.append(1)
        return 1

    def __str__(self):
        ls.append(2)
        return "c'est moi!"

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        ls.append(3)
        print("Wiedersehen")


ls = []


class AIterWrap:
    def __init__(self, obj):
        self._it = iter(obj)

    def __aiter__(self):
        return self

    async def __anext__(self):
        try:
            value = next(self._it)
        except StopIteration:
            raise StopAsyncIteration
        return value


async def a(s, m):
    async with ContextManager() as b:
        print(f"val = {b}")
    await asyncio.sleep(s)
    async for i in AIterWrap(range(0, 2)):
        print(i)
        ls.append(m)
        await asyncio.sleep(1)


loop = asyncio.get_event_loop()
loop.run_until_complete(
    asyncio.wait(
        [a(0, "hello1"), a(0.75, "hello2"), a(1.5, "hello3"), a(2.25, "hello4")]
    )
)


assert ls == [
    1,
    3,
    1,
    3,
    1,
    3,
    1,
    3,
    "hello1",
    "hello2",
    "hello1",
    "hello3",
    "hello2",
    "hello4",
    "hello3",
    "hello4",
]
