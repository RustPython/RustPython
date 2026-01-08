import hashlib
import json
import random

import pvm_host

STATE_KEY = b"staking_state_v1"

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


def _load_state():
    raw = pvm_host.get_state(STATE_KEY)
    if raw is None:
        return None
    return _json_loads(raw)


def _save_state(state):
    pvm_host.set_state(STATE_KEY, _json_dumps(state))


def _add_reward(state, addr, amount):
    if amount <= 0:
        return
    rewards = state["rewards"]
    rewards[addr] = int(rewards.get(addr, 0)) + int(amount)


def _delegations_by_validator(state):
    delegations = state["delegations"]
    by_validator = {}
    for delegator in sorted(delegations):
        entries = delegations[delegator]
        if not isinstance(entries, dict):
            continue
        for validator in sorted(entries):
            amount = int(entries.get(validator, 0))
            if amount <= 0:
                continue
            by_validator.setdefault(validator, []).append((delegator, amount))
    return by_validator


def _validator_weights(state):
    validators = state["validators"]
    weights = {}
    for name in sorted(validators):
        weights[name] = int(validators[name].get("self_stake", 0))
    by_validator = _delegations_by_validator(state)
    for validator, entries in by_validator.items():
        total = sum(amount for _, amount in entries)
        weights[validator] = weights.get(validator, 0) + total
    return weights


def _pick_proposer(weights, seed_int):
    total_weight = sum(weights.values())
    if total_weight <= 0:
        return None
    target = seed_int % total_weight
    running = 0
    for name in sorted(weights):
        running += int(weights[name])
        if target < running:
            return name
    return sorted(weights)[-1]


def _seed_from_ctx(ctx, epoch):
    parts = []
    block_hash = ctx.get("block_hash")
    if isinstance(block_hash, (bytes, bytearray)):
        parts.append(block_hash)
    else:
        parts.append(str(block_hash).encode("utf-8"))
    parts.append(str(ctx.get("block_height", 0)).encode("ascii"))
    parts.append(str(epoch).encode("ascii"))
    digest = hashlib.sha256(b"|".join(parts)).digest()
    return int.from_bytes(digest[:8], "big")


def _demo_actions():
    return [
        {"action": "init", "params": {"inflation": 1200}},
        {
            "action": "register_validator",
            "params": {"validator": "val1", "stake": 500, "commission_bps": 500},
        },
        {
            "action": "register_validator",
            "params": {"validator": "val2", "stake": 350, "commission_bps": 300},
        },
        {
            "action": "delegate",
            "params": {"delegator": "alice", "validator": "val1", "amount": 400},
        },
        {
            "action": "delegate",
            "params": {"delegator": "bob", "validator": "val2", "amount": 250},
        },
        {"action": "distribute", "params": {}},
        {"action": "distribute", "params": {}},
        {"action": "info", "params": {}},
    ]


def _handle_init(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    if state is not None:
        raise ValueError("already initialized")
    inflation = _require_nonneg_int(params.get("inflation", 1000), "inflation")
    state = {
        "version": 1,
        "epoch": 0,
        "inflation": inflation,
        "validators": {},
        "delegations": {},
        "rewards": {},
        "last_proposer": None,
    }
    _save_state(state)
    _emit("staking.init", {"inflation": inflation})
    return state, {"action": "init", "inflation": inflation}


def _handle_register(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    validator = _require_str(params.get("validator"), "validator")
    stake = _require_positive_int(params.get("stake"), "stake")
    commission_bps = _require_int(params.get("commission_bps", 0), "commission_bps")
    if commission_bps < 0 or commission_bps > 10000:
        raise ValueError("commission_bps must be 0..10000")
    entry = state["validators"].get(validator)
    if entry is None:
        entry = {"self_stake": 0, "commission_bps": commission_bps, "active": True}
    else:
        entry["commission_bps"] = commission_bps
    entry["self_stake"] = int(entry.get("self_stake", 0)) + stake
    state["validators"][validator] = entry
    _emit(
        "staking.register",
        {"validator": validator, "stake": stake, "commission_bps": commission_bps},
    )
    return state, {
        "action": "register_validator",
        "validator": validator,
        "self_stake": entry["self_stake"],
    }


def _handle_delegate(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    delegator = _require_str(params.get("delegator"), "delegator")
    validator = _require_str(params.get("validator"), "validator")
    amount = _require_positive_int(params.get("amount"), "amount")
    if validator not in state["validators"]:
        raise ValueError("validator not found")
    entries = state["delegations"].setdefault(delegator, {})
    entries[validator] = int(entries.get(validator, 0)) + amount
    _emit(
        "staking.delegate",
        {"delegator": delegator, "validator": validator, "amount": amount},
    )
    return state, {
        "action": "delegate",
        "delegator": delegator,
        "validator": validator,
        "amount": amount,
    }


def _handle_undelegate(state, params):
    pvm_host.charge_gas(GAS_WRITE)
    delegator = _require_str(params.get("delegator"), "delegator")
    validator = _require_str(params.get("validator"), "validator")
    amount = _require_positive_int(params.get("amount"), "amount")
    entries = state["delegations"].get(delegator)
    if not entries or int(entries.get(validator, 0)) < amount:
        raise ValueError("insufficient delegation")
    new_amount = int(entries.get(validator, 0)) - amount
    if new_amount <= 0:
        entries.pop(validator, None)
    else:
        entries[validator] = new_amount
    if not entries:
        state["delegations"].pop(delegator, None)
    _emit(
        "staking.undelegate",
        {"delegator": delegator, "validator": validator, "amount": amount},
    )
    return state, {
        "action": "undelegate",
        "delegator": delegator,
        "validator": validator,
        "amount": amount,
    }


def _handle_distribute(state, ctx):
    pvm_host.charge_gas(GAS_WRITE)
    epoch = int(state["epoch"])
    weights = _validator_weights(state)
    total_weight = sum(weights.values())
    if total_weight <= 0:
        raise ValueError("no stake available")
    seed_int = _seed_from_ctx(ctx, epoch)
    proposer = _pick_proposer(weights, seed_int)
    inflation = int(state["inflation"])
    reward_map = {}
    distributed = 0
    for name in sorted(weights):
        reward = inflation * int(weights[name]) // total_weight
        reward_map[name] = reward
        distributed += reward
    leftover = inflation - distributed
    if proposer is not None:
        reward_map[proposer] = reward_map.get(proposer, 0) + leftover

    delegations = _delegations_by_validator(state)
    for name in sorted(reward_map):
        reward = int(reward_map[name])
        if reward <= 0:
            continue
        validator_info = state["validators"].get(name, {})
        commission_bps = int(validator_info.get("commission_bps", 0))
        commission = reward * commission_bps // 10000
        _add_reward(state, name, commission)
        remainder = reward - commission
        entries = delegations.get(name, [])
        if not entries or remainder <= 0:
            _add_reward(state, name, remainder)
            continue
        total_delegation = sum(amount for _, amount in entries)
        if total_delegation <= 0:
            _add_reward(state, name, remainder)
            continue
        allocated = 0
        for delegator, amount in entries:
            share = remainder * amount // total_delegation
            allocated += share
            _add_reward(state, delegator, share)
        remainder_left = remainder - allocated
        if remainder_left:
            _add_reward(state, name, remainder_left)

    validator_names = sorted(weights)
    rng = random.Random(seed_int)
    sample_count = min(2, len(validator_names))
    if sample_count:
        sample_validators = sorted(rng.sample(validator_names, sample_count))
    else:
        sample_validators = []

    reward_list = [
        {"validator": name, "reward": reward_map.get(name, 0)}
        for name in sorted(reward_map)
    ]

    state["epoch"] = epoch + 1
    state["last_proposer"] = proposer
    _emit(
        "staking.distribute",
        {"epoch": epoch, "proposer": proposer, "inflation": inflation},
    )
    return state, {
        "action": "distribute",
        "epoch": epoch,
        "proposer": proposer,
        "total_weight": total_weight,
        "validator_rewards": reward_list,
        "sample_validators": sample_validators,
    }


def _handle_info(state):
    pvm_host.charge_gas(GAS_READ)
    return state, {
        "action": "info",
        "epoch": state["epoch"],
        "inflation": state["inflation"],
        "validators": state["validators"],
        "delegations": state["delegations"],
        "rewards": state["rewards"],
        "last_proposer": state["last_proposer"],
    }


def _apply_action(state, action, params, ctx):
    if action == "init":
        return _handle_init(state, params)
    if state is None:
        raise ValueError("not_initialized")
    if action == "register_validator":
        return _handle_register(state, params)
    if action == "delegate":
        return _handle_delegate(state, params)
    if action == "undelegate":
        return _handle_undelegate(state, params)
    if action == "distribute":
        return _handle_distribute(state, ctx)
    if action == "info":
        return _handle_info(state)
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
                "message": "staking rewards demo",
                "actions": [
                    "init",
                    "register_validator",
                    "delegate",
                    "undelegate",
                    "distribute",
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
