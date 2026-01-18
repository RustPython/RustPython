import pvm_host
from pvm_sdk import runner, continuation


def run(coro):
    return coro.send(None)


async def analyze():
    pvm_host.set_state(b"step", b"before")
    cid = continuation.new_cid(None, "llm")
    pvm_host.set_state(b"cid", cid)
    result = await runner.llm("hi")
    pvm_host.set_state(b"step", b"after")
    pvm_host.set_state(b"result", str(result).encode("utf-8"))
    return b"done"


def main(_input):
    return run(analyze())
