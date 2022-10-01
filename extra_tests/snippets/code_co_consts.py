from asyncio import sleep

def f():
    def g():
        return 1

    assert g.__code__.co_consts[0] == None
    return 2

assert f.__code__.co_consts[0] == None

def generator():
  yield 1
  yield 2

assert generator().gi_code.co_consts[0] == None

async def async_f():
  await sleep(1)
  return 1

assert async_f.__code__.co_consts[0] == None

lambda_f = lambda: 0
assert lambda_f.__code__.co_consts[0] == None

class cls:
    def f():
        return 1

assert cls().f.__code__.co_consts[0] == None
