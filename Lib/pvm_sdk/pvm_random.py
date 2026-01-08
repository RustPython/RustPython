import pvm_host

_counter = 0


def _next_block():
    global _counter
    domain = b"random" + _counter.to_bytes(8, "little")
    _counter += 1
    return pvm_host.randomness(domain)


def random():
    block = _next_block()
    value = int.from_bytes(block[:8], "little") >> 11
    return value / (1 << 53)


def randbytes(n):
    out = bytearray()
    while len(out) < n:
        out.extend(_next_block())
    return bytes(out[:n])
