import asyncio_slow as asyncio


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


async def a(s, m):
    async with ContextManager() as b:
        print(f"val = {b}")
    await asyncio.sleep(s)
    for _ in range(0, 2):
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
    "hello1",
    1,
    3,
    1,
    3,
    1,
    3,
    "hello2",
    "hello1",
    "hello3",
    "hello2",
    "hello4",
    "hello3",
    "hello4",
]
