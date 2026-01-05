import pvm_host


def main(input_bytes: bytes) -> bytes:
    ctx = pvm_host.context()
    pvm_host.charge_gas(10)

    current = pvm_host.get_state(b"counter")
    if current is None:
        counter = 1
    else:
        counter = int.from_bytes(current, "little") + 1
    pvm_host.set_state(b"counter", counter.to_bytes(8, "little"))

    pvm_host.emit_event("demo", b"ok")
    payload = input_bytes if input_bytes else b"empty"
    return b"ok:" + payload + b":h=" + str(ctx["block_height"]).encode("ascii")
