import pvm_host
from pvm_sdk import runner, capture, continuation


@runner.continuation
async def analyze(self, msg):
    ctx = capture()
    ctx.value = await runner.llm("hi")
    return ctx.value


def main(input_bytes):
    if input_bytes == b"start":
        cid = continuation.new_cid(None, "analyze")
        pvm_host.set_state(b"cid", cid)
        analyze(None, {})
        return b"started"
    result = input_bytes.decode("utf-8")
    cid = continuation.new_cid(None, "analyze")
    msg = {"cid": cid, "result": result}
    out = analyze__resume(None, msg)
    pvm_host.set_state(b"fsm_result", str(out).encode("utf-8"))
    return b"done"
