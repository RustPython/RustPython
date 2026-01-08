import json

import pvm_host

STATE_KEY = b"dex_state_v1"

GAS_BASE = 5
GAS_READ = 5
GAS_WRITE = 20
GAS_SWAP = 30


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


def _sender_id(ctx):
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


def _require_int(value, name):
    if not isinstance(value, int):
        raise ValueError(f"{name} must be int")
    return value


def _require_positive_int(value, name):
    value = _require_int(value, name)
    if value <= 0:
        raise ValueError(f"{name} must be > 0")
    return value


def _get_balance(state, user, token):
    return int(state["balances"].get(user, {}).get(token, 0))


def _set_balance(state, user, token, amount):
    balances = state["balances"].setdefault(user, {})
    if amount <= 0:
        balances.pop(token, None)
    else:
        balances[token] = amount


def _get_lp(state, user):
    return int(state["lp"].get(user, 0))


def _set_lp(state, user, amount):
    if amount <= 0:
        state["lp"].pop(user, None)
    else:
        state["lp"][user] = amount


def _token_symbol(state, token):
    if token is None:
        raise ValueError("token is required")
    token_str = str(token)
    token_upper = token_str.upper()
    if token_upper == "A":
        return state["token_a"]
    if token_upper == "B":
        return state["token_b"]
    if token_str == state["token_a"] or token_str == state["token_b"]:
        return token_str
    if token_str.lower() == state["token_a"].lower():
        return state["token_a"]
    if token_str.lower() == state["token_b"].lower():
        return state["token_b"]
    raise ValueError("unknown token: " + token_str)


def _public_state(state):
    return {
        "token_a": state["token_a"],
        "token_b": state["token_b"],
        "reserve_a": state["reserve_a"],
        "reserve_b": state["reserve_b"],
        "fee_bps": state["fee_bps"],
        "lp_total": state["lp_total"],
    }


def _isqrt(n):
    if n <= 0:
        return 0
    x = n
    y = (x + 1) // 2
    while y < x:
        x = y
        y = (x + n // x) // 2
    return x


def _amount_out(amount_in, reserve_in, reserve_out, fee_bps):
    if amount_in <= 0:
        return 0
    if reserve_in <= 0 or reserve_out <= 0:
        return 0
    amount_in_with_fee = amount_in * (10000 - fee_bps)
    numerator = amount_in_with_fee * reserve_out
    denom = reserve_in * 10000 + amount_in_with_fee
    return numerator // denom


def _ok(payload):
    payload["ok"] = True
    return _json_dumps(payload)


def _err(code, detail=None):
    out = {"ok": False, "error": code}
    if detail is not None:
        out["detail"] = detail
    return _json_dumps(out)


def _handle_init(params, sender):
    if _load_state() is not None:
        raise ValueError("already initialized")
    token_a = params.get("token_a", "TOKENA")
    token_b = params.get("token_b", "TOKENB")
    if not isinstance(token_a, str) or not isinstance(token_b, str):
        raise ValueError("token names must be strings")
    if token_a == token_b:
        raise ValueError("token_a and token_b must differ")
    fee_bps = params.get("fee_bps", 30)
    fee_bps = _require_int(fee_bps, "fee_bps")
    if fee_bps < 0 or fee_bps > 10000:
        raise ValueError("fee_bps must be 0..10000")
    state = {
        "version": 1,
        "token_a": token_a,
        "token_b": token_b,
        "reserve_a": 0,
        "reserve_b": 0,
        "fee_bps": fee_bps,
        "lp_total": 0,
        "balances": {},
        "lp": {},
    }
    _save_state(state)
    _emit(
        "dex.init",
        {
            "sender": sender,
            "token_a": token_a,
            "token_b": token_b,
            "fee_bps": fee_bps,
        },
    )
    return _ok({"action": "init", "state": _public_state(state)})


def _handle_mint(state, sender, params):
    pvm_host.charge_gas(GAS_WRITE)
    token = _token_symbol(state, params.get("token"))
    amount = _require_positive_int(params.get("amount"), "amount")
    bal = _get_balance(state, sender, token)
    new_bal = bal + amount
    _set_balance(state, sender, token, new_bal)
    _save_state(state)
    _emit("dex.mint", {"sender": sender, "token": token, "amount": amount})
    return _ok(
        {
            "action": "mint",
            "sender": sender,
            "token": token,
            "amount": amount,
            "balance": new_bal,
        }
    )


def _handle_balance(state, sender):
    pvm_host.charge_gas(GAS_READ)
    balances = state["balances"].get(sender, {})
    lp_balance = _get_lp(state, sender)
    return _ok(
        {
            "action": "balance",
            "sender": sender,
            "balances": balances,
            "lp": lp_balance,
        }
    )


def _handle_info(state):
    pvm_host.charge_gas(GAS_READ)
    return _ok({"action": "info", "state": _public_state(state)})


def _handle_quote(state, params):
    pvm_host.charge_gas(GAS_READ)
    token_in = _token_symbol(state, params.get("token_in"))
    amount_in = _require_positive_int(params.get("amount_in"), "amount_in")
    token_a = state["token_a"]
    token_b = state["token_b"]
    if token_in == token_a:
        token_out = token_b
        reserve_in = state["reserve_a"]
        reserve_out = state["reserve_b"]
    else:
        token_out = token_a
        reserve_in = state["reserve_b"]
        reserve_out = state["reserve_a"]
    amount_out = _amount_out(amount_in, reserve_in, reserve_out, state["fee_bps"])
    return _ok(
        {
            "action": "quote",
            "token_in": token_in,
            "token_out": token_out,
            "amount_in": amount_in,
            "amount_out": amount_out,
        }
    )


def _handle_add_liquidity(state, sender, params):
    pvm_host.charge_gas(GAS_WRITE)
    amount_a = _require_positive_int(params.get("amount_a"), "amount_a")
    amount_b = _require_positive_int(params.get("amount_b"), "amount_b")
    min_lp = params.get("min_lp", 0)
    min_lp = _require_int(min_lp, "min_lp")
    if min_lp < 0:
        raise ValueError("min_lp must be >= 0")

    token_a = state["token_a"]
    token_b = state["token_b"]
    bal_a = _get_balance(state, sender, token_a)
    bal_b = _get_balance(state, sender, token_b)
    if bal_a < amount_a or bal_b < amount_b:
        raise ValueError("insufficient balance")

    reserve_a = state["reserve_a"]
    reserve_b = state["reserve_b"]
    lp_total = state["lp_total"]

    if lp_total == 0:
        lp_minted = _isqrt(amount_a * amount_b)
        if lp_minted <= 0:
            raise ValueError("lp_minted is zero")
        used_a = amount_a
        used_b = amount_b
    else:
        if reserve_a <= 0 or reserve_b <= 0:
            raise ValueError("pool reserves are zero")
        lp_from_a = amount_a * lp_total // reserve_a
        lp_from_b = amount_b * lp_total // reserve_b
        lp_minted = min(lp_from_a, lp_from_b)
        if lp_minted <= 0:
            raise ValueError("lp_minted is zero")
        used_a = lp_minted * reserve_a // lp_total
        used_b = lp_minted * reserve_b // lp_total
        if used_a <= 0 or used_b <= 0:
            raise ValueError("used amount is zero")

    if lp_minted < min_lp:
        raise ValueError("lp_minted below min_lp")

    _set_balance(state, sender, token_a, bal_a - used_a)
    _set_balance(state, sender, token_b, bal_b - used_b)

    state["reserve_a"] = reserve_a + used_a
    state["reserve_b"] = reserve_b + used_b
    state["lp_total"] = lp_total + lp_minted

    user_lp = _get_lp(state, sender)
    _set_lp(state, sender, user_lp + lp_minted)

    _save_state(state)
    _emit(
        "dex.add_liquidity",
        {
            "sender": sender,
            "amount_a": used_a,
            "amount_b": used_b,
            "lp_minted": lp_minted,
        },
    )
    return _ok(
        {
            "action": "add_liquidity",
            "sender": sender,
            "amount_a": used_a,
            "amount_b": used_b,
            "lp_minted": lp_minted,
            "lp_total": state["lp_total"],
            "reserves": {"a": state["reserve_a"], "b": state["reserve_b"]},
        }
    )


def _handle_remove_liquidity(state, sender, params):
    pvm_host.charge_gas(GAS_WRITE)
    lp_amount = _require_positive_int(params.get("lp_amount"), "lp_amount")
    min_amount_a = _require_int(params.get("min_amount_a", 0), "min_amount_a")
    min_amount_b = _require_int(params.get("min_amount_b", 0), "min_amount_b")
    if min_amount_a < 0 or min_amount_b < 0:
        raise ValueError("minimums must be >= 0")

    lp_total = state["lp_total"]
    if lp_total <= 0:
        raise ValueError("no liquidity")

    user_lp = _get_lp(state, sender)
    if user_lp < lp_amount:
        raise ValueError("insufficient lp")

    reserve_a = state["reserve_a"]
    reserve_b = state["reserve_b"]
    amount_a = lp_amount * reserve_a // lp_total
    amount_b = lp_amount * reserve_b // lp_total
    if amount_a <= 0 or amount_b <= 0:
        raise ValueError("withdraw amount is zero")
    if amount_a < min_amount_a or amount_b < min_amount_b:
        raise ValueError("withdrawal below minimum")

    state["reserve_a"] = reserve_a - amount_a
    state["reserve_b"] = reserve_b - amount_b
    state["lp_total"] = lp_total - lp_amount

    _set_lp(state, sender, user_lp - lp_amount)

    token_a = state["token_a"]
    token_b = state["token_b"]
    bal_a = _get_balance(state, sender, token_a)
    bal_b = _get_balance(state, sender, token_b)
    _set_balance(state, sender, token_a, bal_a + amount_a)
    _set_balance(state, sender, token_b, bal_b + amount_b)

    _save_state(state)
    _emit(
        "dex.remove_liquidity",
        {
            "sender": sender,
            "lp_amount": lp_amount,
            "amount_a": amount_a,
            "amount_b": amount_b,
        },
    )
    return _ok(
        {
            "action": "remove_liquidity",
            "sender": sender,
            "lp_amount": lp_amount,
            "amount_a": amount_a,
            "amount_b": amount_b,
            "lp_total": state["lp_total"],
            "reserves": {"a": state["reserve_a"], "b": state["reserve_b"]},
        }
    )


def _handle_swap(state, sender, params):
    pvm_host.charge_gas(GAS_SWAP)
    token_in = _token_symbol(state, params.get("token_in"))
    amount_in = _require_positive_int(params.get("amount_in"), "amount_in")
    min_out = _require_int(params.get("min_out", 0), "min_out")
    if min_out < 0:
        raise ValueError("min_out must be >= 0")

    token_a = state["token_a"]
    token_b = state["token_b"]
    if token_in == token_a:
        token_out = token_b
        reserve_in = state["reserve_a"]
        reserve_out = state["reserve_b"]
        reserve_in_key = "reserve_a"
        reserve_out_key = "reserve_b"
    else:
        token_out = token_a
        reserve_in = state["reserve_b"]
        reserve_out = state["reserve_a"]
        reserve_in_key = "reserve_b"
        reserve_out_key = "reserve_a"

    if reserve_in <= 0 or reserve_out <= 0:
        raise ValueError("empty pool")

    bal_in = _get_balance(state, sender, token_in)
    if bal_in < amount_in:
        raise ValueError("insufficient balance")

    amount_out = _amount_out(amount_in, reserve_in, reserve_out, state["fee_bps"])
    if amount_out <= 0:
        raise ValueError("amount_out is zero")
    if amount_out < min_out:
        raise ValueError("amount_out below min_out")
    if amount_out >= reserve_out:
        raise ValueError("insufficient liquidity")

    _set_balance(state, sender, token_in, bal_in - amount_in)
    bal_out = _get_balance(state, sender, token_out)
    _set_balance(state, sender, token_out, bal_out + amount_out)

    state[reserve_in_key] = reserve_in + amount_in
    state[reserve_out_key] = reserve_out - amount_out

    _save_state(state)
    _emit(
        "dex.swap",
        {
            "sender": sender,
            "token_in": token_in,
            "token_out": token_out,
            "amount_in": amount_in,
            "amount_out": amount_out,
        },
    )
    return _ok(
        {
            "action": "swap",
            "sender": sender,
            "token_in": token_in,
            "token_out": token_out,
            "amount_in": amount_in,
            "amount_out": amount_out,
            "reserves": {"a": state["reserve_a"], "b": state["reserve_b"]},
        }
    )


def main(input_bytes):
    pvm_host.charge_gas(GAS_BASE)
    if not input_bytes:
        return _ok(
            {
                "message": "pvm dex demo",
                "actions": [
                    "init",
                    "mint",
                    "add_liquidity",
                    "remove_liquidity",
                    "swap",
                    "quote",
                    "balance",
                    "info",
                ],
            }
        )

    try:
        request = _json_loads(input_bytes)
    except Exception as exc:
        return _err("invalid_json", str(exc))

    action = request.get("action")
    params = request.get("params", {})
    if params is None:
        params = {}
    if not isinstance(params, dict):
        return _err("invalid_input", "params must be object")

    ctx = pvm_host.context()
    sender = _sender_id(ctx)

    try:
        if action == "init":
            return _handle_init(params, sender)

        state = _load_state()
        if state is None:
            return _err("not_initialized")

        if action == "mint":
            return _handle_mint(state, sender, params)
        if action == "balance":
            return _handle_balance(state, sender)
        if action == "info":
            return _handle_info(state)
        if action == "quote":
            return _handle_quote(state, params)
        if action == "add_liquidity":
            return _handle_add_liquidity(state, sender, params)
        if action == "remove_liquidity":
            return _handle_remove_liquidity(state, sender, params)
        if action == "swap":
            return _handle_swap(state, sender, params)
    except Exception as exc:
        return _err("invalid_input", str(exc))

    return _err("unknown_action", str(action))
