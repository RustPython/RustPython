import json

import pvm_host

STATE_KEY = b"actor_transfer_state_v1"

GAS_BASE = 5
GAS_READ = 5
GAS_WRITE = 20


def _json_dumps(value):
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
    ).encode("ascii")


def _json_loads(data):
    return json.loads(data.decode("utf-8"))


def _emit(topic, payload):
    pvm_host.emit_event(topic, _json_dumps(payload))


def _ok(payload):
    payload["ok"] = True
    return _json_dumps(payload)


def _err(code, detail=None):
    out = {"ok": False, "error": code}
    if detail is not None:
        out["detail"] = detail
    return _json_dumps(out)


def _require_int(value, name):
    if not isinstance(value, int):
        raise ValueError(f"{name} must be int")
    return value


def _require_positive_int(value, name):
    value = _require_int(value, name)
    if value <= 0:
        raise ValueError(f"{name} must be > 0")
    return value


def _require_str(value, name):
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be non-empty string")
    return value


def _normalize_user(value, name):
    if value is None:
        raise ValueError(f"{name} required")
    if isinstance(value, str):
        return _require_str(value, name)
    return _require_str(str(value), name)


def _ctx_sender(ctx):
    sender = ctx.get("sender", b"")
    if isinstance(sender, (bytes, bytearray)):
        try:
            return sender.decode("ascii")
        except Exception:
            return sender.hex()
    return str(sender)


def _load_state():
    raw = pvm_host.get_state(STATE_KEY)
    if raw is None:
        return None
    return _json_loads(raw)


def _save_state(state):
    pvm_host.set_state(STATE_KEY, _json_dumps(state))


def _balance_get(state, user):
    return int(state["balances"].get(user, 0))


def _balance_set(state, user, amount):
    if amount <= 0:
        state["balances"].pop(user, None)
    else:
        state["balances"][user] = amount


def _new_state():
    return {"version": 1, "balances": {}}


def _parse_input(input_bytes):
    if not input_bytes:
        return "info", {}
    data = _json_loads(input_bytes)
    if not isinstance(data, dict):
        raise ValueError("input must be object")
    action = data.get("action", "info")
    if not isinstance(action, str) or not action:
        raise ValueError("action must be non-empty string")
    params = data.get("params", {})
    if params is None:
        params = {}
    if not isinstance(params, dict):
        raise ValueError("params must be object")
    return action, params


def _handle_init(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    if state is not None:
        raise ValueError("already initialized")
    balances = params.get("balances", {})
    if balances is None:
        balances = {}
    if not isinstance(balances, dict):
        raise ValueError("balances must be object")
    state = _new_state()
    for user, amount in balances.items():
        user = _normalize_user(user, "user")
        amount = _require_positive_int(amount, "amount")
        _balance_set(state, user, amount)
    _save_state(state)
    _emit("actor.init", {"balances": len(state["balances"])})
    return _ok({"action": "init", "balances": state["balances"]})


def _handle_mint(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    user = params.get("to")
    if user is None:
        user = params.get("user")
    user = _normalize_user(user, "to")
    amount = _require_positive_int(params.get("amount"), "amount")
    new_bal = _balance_get(state, user) + amount
    _balance_set(state, user, new_bal)
    _save_state(state)
    _emit("actor.mint", {"to": user, "amount": amount})
    return _ok({"action": "mint", "to": user, "balance": new_bal})


def _handle_transfer(state, params, ctx_sender):
    pvm_host.charge_gas(GAS_WRITE)
    sender = params.get("from")
    if sender is None:
        sender = ctx_sender
    else:
        sender = _normalize_user(sender, "from")
    if sender != ctx_sender:
        raise ValueError("from must match sender")
    to = _normalize_user(params.get("to"), "to")
    amount = _require_positive_int(params.get("amount"), "amount")
    if sender == to:
        raise ValueError("from and to must differ")
    sender_bal = _balance_get(state, sender)
    if sender_bal < amount:
        raise ValueError("insufficient balance")
    _balance_set(state, sender, sender_bal - amount)
    receiver_bal = _balance_get(state, to) + amount
    _balance_set(state, to, receiver_bal)
    _save_state(state)
    _emit("actor.transfer", {"from": sender, "to": to, "amount": amount})
    return _ok(
        {"action": "transfer", "from": sender, "to": to, "amount": amount}
    )


def _handle_balance(state, params, ctx_sender):
    pvm_host.charge_gas(GAS_READ)
    user = params.get("user")
    if user is None:
        user = ctx_sender
    else:
        user = _normalize_user(user, "user")
    bal = _balance_get(state, user)
    return _ok({"action": "balance", "user": user, "balance": bal})


def _handle_info(state):
    pvm_host.charge_gas(GAS_READ)
    return _ok({"action": "info", "state": state})


def main(input_bytes: bytes) -> bytes:
    pvm_host.charge_gas(GAS_BASE)
    ctx = pvm_host.context()
    ctx_sender = _ctx_sender(ctx)
    try:
        action, params = _parse_input(input_bytes)
        state = _load_state()
        if action == "init":
            return _handle_init(state, params)
        if state is None:
            state = _new_state()
        if action == "mint":
            return _handle_mint(state, params)
        if action == "transfer":
            return _handle_transfer(state, params, ctx_sender)
        if action == "balance":
            return _handle_balance(state, params, ctx_sender)
        if action == "info":
            return _handle_info(state)
        raise ValueError("unknown action")
    except Exception as exc:
        return _err("invalid_request", str(exc))
