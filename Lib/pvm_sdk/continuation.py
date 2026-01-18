import hashlib
import json

import pvm_host


def _encode_value(value):
    if isinstance(value, bytes):
        return {"__bytes__": value.hex()}
    if isinstance(value, bytearray):
        return {"__bytes__": bytes(value).hex()}
    if isinstance(value, dict):
        return {str(k): _encode_value(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_encode_value(v) for v in value]
    if value is None or isinstance(value, (bool, int, str)):
        return value
    raise TypeError("unsupported capture value type")


def _decode_value(value):
    if isinstance(value, dict) and "__bytes__" in value:
        return bytes.fromhex(value["__bytes__"])
    if isinstance(value, dict):
        return {k: _decode_value(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_decode_value(v) for v in value]
    return value


def _encode_json(value):
    try:
        return json.dumps(
            value,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
        ).encode("ascii")
    except AttributeError as exc:
        if "check_circular" not in str(exc):
            raise
        try:
            import importlib
            import json as _json
            importlib.reload(_json)
            return _json.dumps(
                value,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=True,
            ).encode("ascii")
        except Exception:
            raise exc


def _decode_json(data):
    try:
        return json.loads(data.decode("utf-8"))
    except TypeError as exc:
        if "Pattern" not in str(exc):
            raise
        try:
            import importlib
            import re as _re
            import json as _json
            importlib.reload(_re)
            importlib.reload(_json)
            return _json.loads(data.decode("utf-8"))
        except Exception:
            raise exc


class Capture:
    def __init__(self):
        object.__setattr__(self, "_data", {})

    def __getattr__(self, name):
        data = object.__getattribute__(self, "_data")
        if name in data:
            return data[name]
        raise AttributeError(name)

    def __setattr__(self, name, value):
        data = object.__getattribute__(self, "_data")
        data[name] = value

    def to_dict(self):
        return dict(self._data)

    @classmethod
    def from_dict(cls, value):
        inst = cls()
        for k, v in value.items():
            inst._data[k] = v
        return inst


def capture():
    return Capture()


def new_cid(self_obj, name):
    ctx = pvm_host.context()
    seed = b""
    tx_hash = ctx.get("tx_hash")
    if isinstance(tx_hash, (bytes, bytearray)):
        seed += bytes(tx_hash)
    sender = ctx.get("sender")
    if isinstance(sender, (bytes, bytearray)):
        seed += bytes(sender)
    seed += str(name).encode("utf-8")
    return hashlib.sha256(seed).digest()


def _cont_key(cid):
    return b"__continuation:" + cid


def save_cont(cid, state, ctx, handler, timeout_blocks=0, guard_unchanged=None):
    if isinstance(ctx, Capture):
        ctx_dict = ctx.to_dict()
    else:
        ctx_dict = dict(ctx)
    guard_value = guard_unchanged
    if isinstance(guard_value, Capture):
        guard_value = guard_value.to_dict()
    payload = {
        "state": int(state),
        "ctx": _encode_value(ctx_dict),
        "handler": str(handler),
        "timeout_blocks": int(timeout_blocks),
        "guard_unchanged": _encode_value(guard_value),
    }
    pvm_host.set_state(_cont_key(cid), _encode_json(payload))


def load_cont(cid):
    raw = pvm_host.get_state(_cont_key(cid))
    if raw is None:
        raise RuntimeError("continuation state missing")
    data = _decode_json(raw)
    ctx = _decode_value(data.get("ctx") or {})
    data["ctx"] = Capture.from_dict(ctx)
    if "guard_unchanged" in data:
        data["guard_unchanged"] = _decode_value(data["guard_unchanged"])
    return data


def delete_cont(cid):
    pvm_host.delete_state(_cont_key(cid))


def encode_payload(value):
    return _encode_json(_encode_value(value))


def decode_payload(data):
    return _decode_value(_decode_json(data))
