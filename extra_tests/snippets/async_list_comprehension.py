import asyncio

def echo(value):
    return value

async def async_echo(value):
    return value

async def async_in_async():
    return [await async_echo(a) for a in range(5)]

async def sync_in_async():
    return [echo(a) for a in range(5)]

assert asyncio.run(async_in_async()) == [0, 1, 2, 3, 4]
assert asyncio.run(sync_in_async()) == [0, 1, 2, 3, 4]
