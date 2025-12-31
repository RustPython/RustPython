from __future__ import annotations

from pathlib import Path
import os

import rustpython_checkpoint as rpc  # type: ignore

CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))
SEP = "=" * 68

print(SEP)
print("PVM Actor Checkpoint Placement Demo")
print("Flow: run -> #1(function) -> #2(loop) -> #3(if) -> #4(try) -> done")
print("Use --resume to continue after each checkpoint.")
print(SEP)

actor = {
    "actor_id": "actor:alpha",
    "balance": 1200.0,
    "limits": {"daily": 2000.0, "transfer": 600.0},
    "flags": [],
    "history": [],
}

mailbox = [
    {"type": "deposit", "amount": 1200.0, "meta": {"source": "salary"}},
    {"type": "transfer", "to": "actor:beta", "amount": 350.0, "meta": {"note": "rent"}},
    {"type": "transfer", "to": "actor:gamma", "amount": 650.0, "meta": {"note": "equipment"}},
    {"type": "adjust", "amount": -50.0, "meta": {"reason": "fee"}},
    {"type": "query", "fields": ["balance", "flags"]},
    {"type": "noop"},
]

normalized_mailbox = [
    {
        "seq": i,
        **msg,
        "amount": round(msg.get("amount", 0.0), 2),
        "meta": {**msg.get("meta", {}), "batch": "b001"},
    }
    for i, msg in enumerate(mailbox, start=1)
]

summary = {
    "mailbox_size": len(normalized_mailbox),
    "types": sorted({msg["type"] for msg in normalized_mailbox}),
}
actor["history"].append({"event": "bootstrap", **summary})

print(f"[init] actor={actor['actor_id']} balance={actor['balance']}")
print(f"[init] summary={summary}")


def stage_function(state: dict[str, object], messages: list[dict[str, object]]) -> None:
    state["history"].append({"stage": "function", "count": len(messages)})
    state["flags"].append("function:armed")
    print(SEP)
    print("[checkpoint #1] inside function")
    rpc.checkpoint(CHECKPOINT_PATH)
    state["history"].append({"stage": "function", "resume": True})
    print("[resume #1] after function checkpoint")


stage_function(actor, normalized_mailbox)

# import rustpython_checkpoint as rpc  # type: ignore

print(SEP)
print("[2/5] loop stage")
for idx, msg in enumerate(normalized_mailbox):
    if msg["type"] == "transfer" and not actor.get("loop_checkpoint"):
        actor["loop_checkpoint"] = True
        actor["history"].append({"stage": "loop", "seq": idx})
        print("[checkpoint #2] inside loop")
        rpc.checkpoint(CHECKPOINT_PATH)
        print("[resume #2] after loop checkpoint")

    match msg:
        case {"type": "deposit", "amount": amt}:
            actor["balance"] = round(actor["balance"] + amt, 2)
        case {"type": "transfer", "amount": amt, "to": target}:
            if actor["balance"] >= amt:
                actor["balance"] = round(actor["balance"] - amt, 2)
            else:
                actor["flags"].append(f"overdraft:{target}")
        case {"type": "adjust", "amount": amt}:
            actor["balance"] = round(actor["balance"] + amt, 2)
        case {"type": "query", "fields": fields}:
            snapshot = {field: actor.get(field) for field in fields}
            actor["history"].append({"stage": "query", "snapshot": snapshot})
        case {"type": "noop"}:
            actor["flags"].append("noop")
        case _:
            actor["flags"].append("unknown")

# import rustpython_checkpoint as rpc  # type: ignore

print(SEP)
print("[3/5] if stage")
if actor["balance"] >= 0 and not actor.get("if_checkpoint"):
    actor["if_checkpoint"] = True
    actor["history"].append({"stage": "if", "balance": actor["balance"]})
    print("[checkpoint #3] inside if")
    rpc.checkpoint(CHECKPOINT_PATH)
    actor["flags"].append("if_resumed")
    print("[resume #3] after if checkpoint")

# import rustpython_checkpoint as rpc  # type: ignore

print(SEP)
print("[4/5] try/except stage")
try:
    if not actor.get("try_checkpoint"):
        actor["try_checkpoint"] = True
        actor["history"].append({"stage": "try"})
        print("[checkpoint #4] inside try")
        rpc.checkpoint(CHECKPOINT_PATH)
        print("[resume #4] after try checkpoint")
    raise ValueError("demo")
except ValueError as exc:
    actor["flags"].append(f"handled:{exc}")

print(SEP)
print("[5/5] final report")
report = {
    "actor_id": actor["actor_id"],
    "balance": actor["balance"],
    "flags": actor["flags"],
    "history_tail": actor["history"][-4:],
}
print(f"  report={report}")

if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
print("[done] checkpoint file removed; next run starts fresh")
