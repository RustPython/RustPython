import sys
import asyncio
import unittest


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


if sys.platform.startswith("win"):
    SLEEP_UNIT = 1.0
else:
    SLEEP_UNIT = 0.1

async def a(s, m):
    async with ContextManager() as b:
        print(f"val = {b}")
    await asyncio.sleep(s)
    async for i in AIterWrap(range(0, 2)):
        print(i)
        ls.append(m)
        await asyncio.sleep(SLEEP_UNIT)



async def main():
    tasks = [
        asyncio.create_task(c)
        for c in [a(SLEEP_UNIT * 0, "hello1"), a(SLEEP_UNIT * 1, "hello2"), a(SLEEP_UNIT * 2, "hello3"), a(SLEEP_UNIT * 3, "hello4")]
    ]
    await asyncio.wait(tasks)


ls = []
asyncio.run(main(), debug=True)

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


if sys.version_info < (3, 11, 0):

    class TestAsyncWith(unittest.TestCase):
        def testAenterAttributeError1(self):
            class LacksAenter(object):
                async def __aexit__(self, *exc):
                    pass

            async def foo():
                async with LacksAenter():
                    pass

            with self.assertRaisesRegex(AttributeError, "__aenter__"):
                foo().send(None)

        def testAenterAttributeError2(self):
            class LacksAenterAndAexit(object):
                pass

            async def foo():
                async with LacksAenterAndAexit():
                    pass

            with self.assertRaisesRegex(AttributeError, "__aenter__"):
                foo().send(None)

        def testAexitAttributeError(self):
            class LacksAexit(object):
                async def __aenter__(self):
                    pass

            async def foo():
                async with LacksAexit():
                    pass

            with self.assertRaisesRegex(AttributeError, "__aexit__"):
                foo().send(None)


if __name__ == "__main__":
    unittest.main()
