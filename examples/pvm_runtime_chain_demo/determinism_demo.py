import hashlib
import json
import random
import struct
import time

import pvm_host


def _hex(b):
    return b.hex() if isinstance(b, (bytes, bytearray)) else None


def main(input_bytes):
    ctx = pvm_host.context()

    # Deterministic state and event usage (idempotent write).
    state_key = b"demo:state"
    state_value = hashlib.sha256(input_bytes).digest()
    pvm_host.set_state(state_key, state_value)
    current_state = pvm_host.get_state(state_key)
    pvm_host.emit_event("demo", state_value)

    # Deterministic randomness/time from host context.
    rand_bytes = random.randbytes(16)
    timestamp_ns = time.time_ns()

    # Use whitelisted stdlib modules.
    packed_len = struct.pack("<I", len(input_bytes))

    blocked = {}
    for name in ("os", "socket", "pathlib", "sys"):
        try:
            __import__(name)
            blocked[name] = "allowed"
        except Exception as exc:
            blocked[name] = type(exc).__name__

    try:
        open("forbidden.txt", "wb").write(b"x")
        blocked["open"] = "allowed"
    except Exception as exc:
        blocked["open"] = type(exc).__name__

    result = {
        "input_hex": input_bytes.hex(),
        "timestamp_ns": timestamp_ns,
        "random_hex": rand_bytes.hex(),
        "packed_len_hex": packed_len.hex(),
        "state_hex": _hex(current_state),
        "event_hex": state_value.hex(),
        "ctx": {
            "block_height": ctx.get("block_height"),
            "block_hash": _hex(ctx.get("block_hash")),
            "tx_hash": _hex(ctx.get("tx_hash")),
            "sender": _hex(ctx.get("sender")),
            "timestamp_ms": ctx.get("timestamp_ms"),
        },
        "module_time": getattr(time, "__name__", ""),
        "module_random": getattr(random, "__name__", ""),
        "blocked": blocked,
    }

    return json.dumps(result, sort_keys=True).encode()
