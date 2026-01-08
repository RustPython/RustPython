import hashlib
import json
import random

import pvm_host

STATE_KEY = b"payroll_state_v1"

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


def _require_str(value, name):
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be non-empty string")
    return value


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


def _demo_actions():
    return [
        {
            "action": "init",
            "params": {"fee_bps": 25, "treasury": "treasury"},
        },
        {"action": "credit", "params": {"user": "alice", "amount": 2000}},
        {"action": "credit", "params": {"user": "carol", "amount": 1500}},
        {
            "action": "process_batch",
            "params": {
                "batch_id": "payroll-2024-09",
                "transfers": [
                    {"from": "alice", "to": "bob", "amount": 300},
                    {"from": "alice", "to": "dora", "amount": 250},
                    {"from": "carol", "to": "erin", "amount": 400},
                    {"from": "carol", "to": "frank", "amount": 200},
                ],
            },
        },
        {"action": "snapshot", "params": {}},
    ]


def _handle_init(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    if state is not None:
        raise ValueError("already initialized")
    fee_bps = _require_int(params.get("fee_bps", 25), "fee_bps")
    if fee_bps < 0 or fee_bps > 10000:
        raise ValueError("fee_bps must be 0..10000")
    treasury = _require_str(params.get("treasury", "treasury"), "treasury")
    state = {
        "version": 1,
        "fee_bps": fee_bps,
        "treasury": treasury,
        "balances": {},
        "batches": {},
        "ledger": [],
        "sequence": 1,
    }
    _save_state(state)
    _emit("payroll.init", {"fee_bps": fee_bps, "treasury": treasury})
    return state, {"action": "init", "fee_bps": fee_bps, "treasury": treasury}


def _handle_credit(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    user = _require_str(params.get("user"), "user")
    amount = _require_positive_int(params.get("amount"), "amount")
    new_bal = _balance_get(state, user) + amount
    _balance_set(state, user, new_bal)
    _emit("payroll.credit", {"user": user, "amount": amount})
    return state, {"action": "credit", "user": user, "balance": new_bal}


def _handle_process_batch(state, params, ctx):
    transfers = params.get("transfers")
    if not isinstance(transfers, list) or not transfers:
        raise ValueError("transfers must be non-empty list")
    pvm_host.charge_gas(GAS_WRITE + len(transfers))
    batch_id = _require_str(params.get("batch_id"), "batch_id")
    if batch_id in state["batches"]:
        raise ValueError("batch_id already processed")
    fee_bps = int(state["fee_bps"])
    required = {}
    normalized = []
    total_amount = 0
    total_fee = 0
    for idx, entry in enumerate(transfers):
        if not isinstance(entry, dict):
            raise ValueError("transfer entry must be object")
        sender = _require_str(entry.get("from"), "from")
        recipient = _require_str(entry.get("to"), "to")
        amount = _require_positive_int(entry.get("amount"), "amount")
        fee = amount * fee_bps // 10000
        normalized.append(
            {
                "index": idx,
                "from": sender,
                "to": recipient,
                "amount": amount,
                "fee": fee,
            }
        )
        required[sender] = required.get(sender, 0) + amount + fee
        total_amount += amount
        total_fee += fee

    for sender, needed in required.items():
        if _balance_get(state, sender) < needed:
            raise ValueError(f"insufficient balance: {sender}")

    digest = hashlib.sha256()
    for entry in normalized:
        line = (
            f"{entry['index']}:{entry['from']}->{entry['to']}:"
            f"{entry['amount']}:{entry['fee']}"
        )
        digest.update(line.encode("utf-8"))
    digest_hex = digest.hexdigest()

    seed_material = f"{digest_hex}:{ctx.get('block_height', 0)}"
    seed_hex = hashlib.sha256(seed_material.encode("utf-8")).hexdigest()[:16]
    rng = random.Random(int(seed_hex, 16))
    sample_count = min(2, len(normalized))
    if sample_count:
        sample_indices = sorted(
            rng.sample(range(len(normalized)), sample_count)
        )
    else:
        sample_indices = []
    sample_hash = hashlib.sha256()
    for idx in sample_indices:
        entry = normalized[idx]
        line = (
            f"{entry['index']}:{entry['from']}:{entry['to']}:"
            f"{entry['amount']}:{entry['fee']}"
        )
        sample_hash.update(line.encode("utf-8"))

    treasury = state["treasury"]
    ledger = state["ledger"]
    sequence = int(state["sequence"])
    for entry in normalized:
        sender = entry["from"]
        recipient = entry["to"]
        amount = entry["amount"]
        fee = entry["fee"]
        _balance_set(
            state, sender, _balance_get(state, sender) - amount - fee
        )
        _balance_set(
            state, recipient, _balance_get(state, recipient) + amount
        )
        _balance_set(state, treasury, _balance_get(state, treasury) + fee)
        ledger.append(
            {
                "id": sequence,
                "batch_id": batch_id,
                "index": entry["index"],
                "from": sender,
                "to": recipient,
                "amount": amount,
                "fee": fee,
            }
        )
        sequence += 1
        _emit(
            "payroll.transfer",
            {
                "batch_id": batch_id,
                "index": entry["index"],
                "from": sender,
                "to": recipient,
                "amount": amount,
                "fee": fee,
            },
        )
    state["sequence"] = sequence
    state["batches"][batch_id] = {
        "count": len(normalized),
        "total_amount": total_amount,
        "total_fee": total_fee,
        "digest": digest_hex,
        "sample_digest": sample_hash.hexdigest(),
    }
    _emit(
        "payroll.batch",
        {
            "batch_id": batch_id,
            "count": len(normalized),
            "total_amount": total_amount,
            "total_fee": total_fee,
        },
    )
    return state, {
        "action": "process_batch",
        "batch_id": batch_id,
        "count": len(normalized),
        "total_amount": total_amount,
        "total_fee": total_fee,
        "digest": digest_hex,
        "audit_sample_indices": sample_indices,
    }


def _handle_snapshot(state):
    pvm_host.charge_gas(GAS_READ)
    return state, {
        "action": "snapshot",
        "balances": state["balances"],
        "batch_count": len(state["batches"]),
        "ledger_len": len(state["ledger"]),
    }


def _handle_balance(state, params):
    pvm_host.charge_gas(GAS_READ)
    user = params.get("user")
    if user is None:
        return state, {"action": "balance", "balances": state["balances"]}
    user = _require_str(user, "user")
    return state, {"action": "balance", "user": user, "balance": _balance_get(state, user)}


def _handle_batch_info(state, params):
    pvm_host.charge_gas(GAS_READ)
    batch_id = _require_str(params.get("batch_id"), "batch_id")
    info = state["batches"].get(batch_id)
    if info is None:
        raise ValueError("batch_id not found")
    return state, {"action": "batch_info", "batch_id": batch_id, "info": info}


def _apply_action(state, action, params, ctx):
    if action == "init":
        return _handle_init(state, params)
    if state is None:
        raise ValueError("not_initialized")
    if action == "credit":
        return _handle_credit(state, params)
    if action == "process_batch":
        return _handle_process_batch(state, params, ctx)
    if action == "snapshot":
        return _handle_snapshot(state)
    if action == "balance":
        return _handle_balance(state, params)
    if action == "batch_info":
        return _handle_batch_info(state, params)
    raise ValueError(f"unknown_action: {action}")


def _run_actions(state, actions, ctx):
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
        state, summary = _apply_action(state, action, params, ctx)
        _save_state(state)
        results.append(summary)
    return state, results


def main(input_bytes):
    pvm_host.charge_gas(GAS_BASE)
    if not input_bytes:
        return _ok(
            {
                "message": "batch payroll demo",
                "actions": [
                    "init",
                    "credit",
                    "process_batch",
                    "snapshot",
                    "balance",
                    "batch_info",
                ],
                "hint": "pass 'demo' or a JSON object",
            }
        )

    try:
        text = input_bytes.decode("utf-8")
    except Exception as exc:
        return _err("invalid_input", str(exc))

    ctx = pvm_host.context()

    if text.strip().lower() == "demo":
        actions = _demo_actions()
        try:
            state = _load_state()
            state, results = _run_actions(state, actions, ctx)
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
            state, results = _run_actions(state, actions, ctx)
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
        state, summary = _apply_action(state, action, params, ctx)
        _save_state(state)
        return _ok(summary, state)
    except Exception as exc:
        return _err("invalid_input", str(exc))
