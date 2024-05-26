import asyncio
from types import GeneratorType, AsyncGeneratorType


async def f_async(x):
    await asyncio.sleep(0.001)
    return x


def f_iter():
    for i in range(5):
        yield i


async def f_aiter():
    for i in range(5):
        await asyncio.sleep(0.001)
        yield i


async def run_async():
    # list
    x = [i for i in range(5)]
    assert isinstance(x, list)
    for i, e in enumerate(x):
        assert e == i

    x = [await f_async(i) for i in range(5)]
    assert isinstance(x, list)
    for i, e in enumerate(x):
        assert e == i

    x = [e async for e in f_aiter()]
    assert isinstance(x, list)
    for i, e in enumerate(x):
        assert e == i

    x = [await f_async(i) async for i in f_aiter()]
    assert isinstance(x, list)
    for i, e in enumerate(x):
        assert e == i

    # set
    x = {i for i in range(5)}
    assert isinstance(x, set)
    for e in x:
        assert e in range(5)
    assert x == {0, 1, 2, 3, 4}

    x = {await f_async(i) for i in range(5)}
    assert isinstance(x, set)
    for e in x:
        assert e in range(5)
    assert x == {0, 1, 2, 3, 4}

    x = {e async for e in f_aiter()}
    assert isinstance(x, set)
    for e in x:
        assert e in range(5)
    assert x == {0, 1, 2, 3, 4}

    x = {await f_async(i) async for i in f_aiter()}
    assert isinstance(x, set)
    for e in x:
        assert e in range(5)
    assert x == {0, 1, 2, 3, 4}

    # dict
    x = {i: i for i in range(5)}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {await f_async(i): i for i in range(5)}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {i: await f_async(i) for i in range(5)}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {await f_async(i): await f_async(i) for i in range(5)}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {i: i async for i in f_aiter()}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {await f_async(i): i async for i in f_aiter()}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {i: await f_async(i) async for i in f_aiter()}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    x = {await f_async(i): await f_async(i) async for i in f_aiter()}
    assert isinstance(x, dict)
    for k, v in x.items():
        assert k == v
    assert x == {0: 0, 1: 1, 2: 2, 3: 3, 4: 4}

    # generator
    x = (i for i in range(5))
    assert isinstance(x, GeneratorType)
    for i, e in enumerate(x):
        assert e == i

    x = (await f_async(i) for i in range(5))
    assert isinstance(x, AsyncGeneratorType)
    i = 0
    async for e in x:
        assert e == i
        i += 1

    x = (e async for e in f_aiter())
    assert isinstance(x, AsyncGeneratorType)
    i = 0
    async for e in x:
        assert i == e
        i += 1

    x = (await f_async(i) async for i in f_aiter())
    assert isinstance(x, AsyncGeneratorType)
    i = 0
    async for e in x:
        assert i == e
        i += 1


def run_sync():
    async def test_async_for(x):
        i = 0
        async for e in x:
            assert e == i
            i += 1

    x = (i for i in range(5))
    assert isinstance(x, GeneratorType)
    for i, e in enumerate(x):
        assert e == i

    x = (await f_async(i) for i in range(5))
    assert isinstance(x, AsyncGeneratorType)
    asyncio.run(test_async_for(x), debug=True)

    x = (e async for e in f_aiter())
    assert isinstance(x, AsyncGeneratorType)
    asyncio.run(test_async_for(x), debug=True)

    x = (await f_async(i) async for i in f_aiter())
    assert isinstance(x, AsyncGeneratorType)
    asyncio.run(test_async_for(x), debug=True)


asyncio.run(run_async(), debug=True)
run_sync()
