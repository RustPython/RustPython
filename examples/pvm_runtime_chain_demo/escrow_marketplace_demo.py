import hashlib
import json

import pvm_host

STATE_KEY = b"escrow_state_v1"

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


def _state_hash(state):
    return hashlib.sha256(_json_dumps(state)).hexdigest()


def _ok(payload, state=None):
    payload["ok"] = True
    if state is not None:
        payload["state_hash"] = _state_hash(state)
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


def _require_nonneg_int(value, name):
    value = _require_int(value, name)
    if value < 0:
        raise ValueError(f"{name} must be >= 0")
    return value


def _require_str(value, name):
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be non-empty string")
    return value


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


def _order_list(state):
    orders = state["orders"]
    out = []
    for key in sorted(orders, key=lambda k: int(k)):
        out.append(orders[key])
    return out


def _demo_actions():
    return [
        {
            "action": "init",
            "params": {"fee_bps": 50, "treasury": "treasury"},
        },
        {"action": "deposit", "params": {"user": "alice", "amount": 1000}},
        {"action": "deposit", "params": {"user": "bob", "amount": 800}},
        {
            "action": "list",
            "params": {
                "seller": "alice",
                "item": "camera",
                "price": 300,
                "expires_in": 5,
            },
        },
        {"action": "fund", "params": {"order_id": 1, "buyer": "bob"}},
        {"action": "release", "params": {"order_id": 1, "actor": "alice"}},
        {
            "action": "list",
            "params": {
                "seller": "bob",
                "item": "bike",
                "price": 200,
                "expires_in": 0,
            },
        },
        {"action": "cancel", "params": {"order_id": 2}},
        {"action": "info", "params": {}},
    ]


def _handle_init(params, ctx):
    pvm_host.charge_gas(GAS_WRITE)
    if _load_state() is not None:
        raise ValueError("already initialized")
    fee_bps = _require_int(params.get("fee_bps", 30), "fee_bps")
    if fee_bps < 0 or fee_bps > 10000:
        raise ValueError("fee_bps must be 0..10000")
    treasury = _require_str(params.get("treasury", "treasury"), "treasury")
    state = {
        "version": 1,
        "fee_bps": fee_bps,
        "treasury": treasury,
        "next_order_id": 1,
        "balances": {},
        "orders": {},
    }
    _save_state(state)
    _emit("escrow.init", {"fee_bps": fee_bps, "treasury": treasury})
    return state, {
        "action": "init",
        "fee_bps": fee_bps,
        "treasury": treasury,
    }


def _handle_deposit(state, params, ctx_sender):
    pvm_host.charge_gas(GAS_WRITE)
    user = params.get("user")
    if user is None:
        user = ctx_sender
    user = _require_str(user, "user")
    amount = _require_positive_int(params.get("amount"), "amount")
    new_bal = _balance_get(state, user) + amount
    _balance_set(state, user, new_bal)
    _emit("escrow.deposit", {"user": user, "amount": amount})
    return state, {"action": "deposit", "user": user, "balance": new_bal}


def _handle_list(state, params, ctx_sender, ctx):
    pvm_host.charge_gas(GAS_WRITE)
    seller = params.get("seller")
    if seller is None:
        seller = ctx_sender
    seller = _require_str(str(seller), "seller")
    item = _require_str(str(params.get("item", "item")), "item")
    price = _require_positive_int(params.get("price"), "price")
    expires_in = _require_nonneg_int(params.get("expires_in", 5), "expires_in")
    height = int(ctx.get("block_height", 0))
    order_id = state["next_order_id"]
    state["next_order_id"] = order_id + 1
    order = {
        "id": order_id,
        "item": item,
        "seller": seller,
        "buyer": None,
        "price": price,
        "fee": 0,
        "escrow": 0,
        "status": "listed",
        "created_height": height,
        "expires_at": height + expires_in,
    }
    state["orders"][str(order_id)] = order
    _emit(
        "escrow.list",
        {"order_id": order_id, "seller": seller, "price": price},
    )
    return state, {"action": "list", "order": order}


def _handle_fund(state, params, ctx_sender, ctx):
    pvm_host.charge_gas(GAS_WRITE)
    order_id = _require_int(params.get("order_id"), "order_id")
    key = str(order_id)
    order = state["orders"].get(key)
    if order is None:
        raise ValueError("unknown order_id")
    if order["status"] != "listed":
        raise ValueError("order not listed")
    height = int(ctx.get("block_height", 0))
    if height >= order["expires_at"]:
        raise ValueError("order expired")
    buyer = params.get("buyer")
    if buyer is None:
        buyer = ctx_sender
    buyer = _require_str(str(buyer), "buyer")
    price = int(order["price"])
    fee = price * int(state["fee_bps"]) // 10000
    escrow_amount = price - fee
    if _balance_get(state, buyer) < price:
        raise ValueError("insufficient balance")
    _balance_set(state, buyer, _balance_get(state, buyer) - price)
    treasury = state["treasury"]
    _balance_set(state, treasury, _balance_get(state, treasury) + fee)
    order["status"] = "funded"
    order["buyer"] = buyer
    order["fee"] = fee
    order["escrow"] = escrow_amount
    order["funded_height"] = height
    _emit(
        "escrow.fund",
        {"order_id": order_id, "buyer": buyer, "escrow": escrow_amount},
    )
    return state, {"action": "fund", "order_id": order_id, "buyer": buyer}


def _handle_release(state, params, ctx_sender, ctx):
    pvm_host.charge_gas(GAS_WRITE)
    order_id = _require_int(params.get("order_id"), "order_id")
    key = str(order_id)
    order = state["orders"].get(key)
    if order is None:
        raise ValueError("unknown order_id")
    if order["status"] != "funded":
        raise ValueError("order not funded")
    actor = params.get("actor")
    if actor is None:
        actor = ctx_sender
    actor = _require_str(str(actor), "actor")
    if actor != order["seller"]:
        raise ValueError("only seller can release")
    seller = order["seller"]
    escrow_amount = int(order["escrow"])
    _balance_set(state, seller, _balance_get(state, seller) + escrow_amount)
    order["status"] = "released"
    order["released_height"] = int(ctx.get("block_height", 0))
    _emit(
        "escrow.release",
        {"order_id": order_id, "seller": seller, "amount": escrow_amount},
    )
    return state, {"action": "release", "order_id": order_id, "seller": seller}


def _handle_cancel(state, params, ctx):
    pvm_host.charge_gas(GAS_WRITE)
    order_id = _require_int(params.get("order_id"), "order_id")
    key = str(order_id)
    order = state["orders"].get(key)
    if order is None:
        raise ValueError("unknown order_id")
    height = int(ctx.get("block_height", 0))
    if height < order["expires_at"]:
        raise ValueError("order not expired")
    if order["status"] == "listed":
        order["status"] = "cancelled"
        order["cancelled_height"] = height
        _emit("escrow.cancel", {"order_id": order_id})
        return state, {"action": "cancel", "order_id": order_id}
    if order["status"] == "funded":
        buyer = order.get("buyer")
        refund = int(order.get("escrow", 0))
        if buyer:
            _balance_set(state, buyer, _balance_get(state, buyer) + refund)
        order["status"] = "refunded"
        order["refunded_height"] = height
        _emit("escrow.refund", {"order_id": order_id, "amount": refund})
        return state, {"action": "refund", "order_id": order_id}
    raise ValueError("order not cancellable")


def _handle_info(state):
    pvm_host.charge_gas(GAS_READ)
    return state, {
        "action": "info",
        "fee_bps": state["fee_bps"],
        "treasury": state["treasury"],
        "order_count": len(state["orders"]),
        "balances": state["balances"],
    }


def _handle_balance(state, params):
    pvm_host.charge_gas(GAS_READ)
    user = params.get("user")
    if user is None:
        return state, {"action": "balance", "balances": state["balances"]}
    user = _require_str(user, "user")
    return state, {
        "action": "balance",
        "user": user,
        "balance": _balance_get(state, user),
    }


def _handle_list_orders(state):
    pvm_host.charge_gas(GAS_READ)
    return state, {"action": "list_orders", "orders": _order_list(state)}


def _apply_action(state, action, params, ctx, ctx_sender):
    if action == "init":
        return _handle_init(params, ctx)
    if state is None:
        raise ValueError("not_initialized")
    if action == "deposit":
        return _handle_deposit(state, params, ctx_sender)
    if action == "list":
        return _handle_list(state, params, ctx_sender, ctx)
    if action == "fund":
        return _handle_fund(state, params, ctx_sender, ctx)
    if action == "release":
        return _handle_release(state, params, ctx_sender, ctx)
    if action == "cancel":
        return _handle_cancel(state, params, ctx)
    if action == "info":
        return _handle_info(state)
    if action == "balance":
        return _handle_balance(state, params)
    if action == "list_orders":
        return _handle_list_orders(state)
    raise ValueError(f"unknown_action: {action}")


def _run_actions(state, actions, ctx, ctx_sender):
    results = []
    for step in actions:
        if not isinstance(step, dict):
            raise ValueError("action entry must be object")
        action = step.get("action")
        params = step.get("params", {})
        if params is None:
            params = {}
        if not isinstance(params, dict):
            raise ValueError("params must be object")
        state, summary = _apply_action(state, action, params, ctx, ctx_sender)
        _save_state(state)
        results.append(summary)
    return state, results


def main(input_bytes):
    pvm_host.charge_gas(GAS_BASE)
    if not input_bytes:
        return _ok(
            {
                "message": "escrow marketplace demo",
                "actions": [
                    "init",
                    "deposit",
                    "list",
                    "fund",
                    "release",
                    "cancel",
                    "balance",
                    "list_orders",
                    "info",
                ],
                "hint": "pass 'demo' or a JSON object",
            }
        )

    try:
        text = input_bytes.decode("utf-8")
    except Exception as exc:
        return _err("invalid_input", str(exc))

    ctx = pvm_host.context()
    ctx_sender = _ctx_sender(ctx)

    if text.strip().lower() == "demo":
        actions = _demo_actions()
        try:
            state = _load_state()
            state, results = _run_actions(state, actions, ctx, ctx_sender)
            return _ok({"action": "batch", "results": results}, state)
        except Exception as exc:
            return _err("invalid_input", str(exc))

    try:
        request = _json_loads(input_bytes)
    except Exception as exc:
        return _err("invalid_json", str(exc))

    if not isinstance(request, dict):
        return _err("invalid_input", "input must be object")

    if "actions" in request:
        actions = request.get("actions")
        if not isinstance(actions, list):
            return _err("invalid_input", "actions must be list")
        try:
            state = _load_state()
            state, results = _run_actions(state, actions, ctx, ctx_sender)
            return _ok({"action": "batch", "results": results}, state)
        except Exception as exc:
            return _err("invalid_input", str(exc))

    action = request.get("action")
    params = request.get("params", {})
    if params is None:
        params = {}
    if not isinstance(params, dict):
        return _err("invalid_input", "params must be object")

    try:
        state = _load_state()
        state, summary = _apply_action(state, action, params, ctx, ctx_sender)
        _save_state(state)
        return _ok(summary, state)
    except Exception as exc:
        return _err("invalid_input", str(exc))
