from __future__ import annotations

from pathlib import Path

import rustpython_checkpoint as rpc  # type: ignore

# Checkpoint file path as a string to keep it serializable.
CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))
SEP = "-" * 60

# Phase 1: prepare input data and a minimal run log.
print("=" * 60)
print(" PVM Breakpoint/Resume Demo")
print(" Idea: VM saves state at checkpoints and exits.")
print(" Run flow: 1) run -> stop #1, 2) resume -> stop #2, 3) resume -> finish")
print("=" * 60)
print("[1/3] build input state (ai agent runs)")
print("  action: load run telemetry, set cost/latency thresholds, and precompute summary stats")
runs = [
    {"id": "job-001", "agent": "planner", "tokens": 640, "tool_calls": 2, "latency_ms": 820},
    {"id": "job-002", "agent": "coder", "tokens": 1480, "tool_calls": 5, "latency_ms": 2400},
    {"id": "job-003", "agent": "reviewer", "tokens": 520, "tool_calls": 1, "latency_ms": 610},
]
cost_per_1k_tokens = 0.03
alert_latency_ms = 2000
run_log = ["loaded runs", f"alert_latency_ms={alert_latency_ms}"]
summary = {
    "run_count": len(runs),
    "total_tokens": sum(item["tokens"] for item in runs),
    "total_latency_ms": sum(item["latency_ms"] for item in runs),
}
for item in runs:
    print(
        "  run {id} agent={agent} tokens={tokens} tool_calls={tool_calls} "
        "latency_ms={latency_ms}".format(**item)
    )
print(f"  cost_per_1k_tokens={cost_per_1k_tokens} alert_latency_ms={alert_latency_ms}")
print(f"  summary={summary}")

# Breakpoint 1: must be a standalone statement.
print(SEP)
print("[checkpoint #1] VM snapshot saved; process exits now")
print("  note: next run with --resume continues from the next line")
print(SEP)
rpc.checkpoint(CHECKPOINT_PATH)

# Re-import after resume so the next checkpoint works.
import rustpython_checkpoint as rpc  # type: ignore

# Phase 2: derive alerts and billing info from restored state.
print("[2/3] resumed after checkpoint #1")
print("  action: flag slow runs, compute per-run cost, aggregate totals, and append run log")
alerts = [item["id"] for item in runs if item["latency_ms"] >= alert_latency_ms]
costs = [
    {"id": item["id"], "cost": round((item["tokens"] / 1000) * cost_per_1k_tokens, 4)}
    for item in runs
]
total_cost = round(sum(item["cost"] for item in costs), 4)
run_log.append(f"alerts={alerts}")
run_log.append(f"total_cost={total_cost}")
print(f"  alerts={alerts}")
print(f"  costs={costs} total_cost={total_cost}")
print(f"  log={run_log}")

# Breakpoint 2: save state again and exit; next run continues below.
print(SEP)
print("[checkpoint #2] VM snapshot saved; process exits now")
print("  note: next run with --resume continues from the next line")
print(SEP)
rpc.checkpoint(CHECKPOINT_PATH)

# After resume, prepare cleanup utilities.
import os

# Phase 3: produce a final report and clean up the checkpoint file.
print(SEP)
print("[3/3] resumed after checkpoint #2")
print("  action: finalize report and cleanup snapshot")
report = {
    "summary": summary,
    "alerts": alerts,
    "costs": costs,
    "total_cost": total_cost,
    "status": "ready",
}
run_log.append("report_ready")
print(f"  report={report}")
print(f"  log={run_log}")
print(SEP)

# Clean up the checkpoint file so the next run starts fresh.
if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
print("[done] checkpoint file removed; next run starts fresh")
