from __future__ import annotations

from pathlib import Path

import rustpython_checkpoint as rpc  # type: ignore
import os

# Checkpoint file path as a string to keep it serializable.
CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))
SEP = "-" * 60

# Phase 1: prepare input data and a minimal run log.
print("=" * 60)
print(" PVM Breakpoint/Resume Demo")
print(" Idea: VM saves state at checkpoints and exits.")
print(" Run flow: 1) run -> stop #1, 2) resume -> stop #2, 3) resume -> finish")
print("=" * 60)
print("[1/3] build input state (trading day snapshot)")
print("  action: load orders, prices, and risk limits; precompute exposure summary")
orders = [
    {"id": "ord-001", "symbol": "AAPL", "side": "BUY", "qty": 120, "limit": 192.10},
    {"id": "ord-002", "symbol": "MSFT", "side": "SELL", "qty": 80, "limit": 411.50},
    {"id": "ord-003", "symbol": "NVDA", "side": "BUY", "qty": 60, "limit": 122.30},
]
prices = {"AAPL": 192.25, "MSFT": 411.10, "NVDA": 122.60}
max_order_notional = 25000.0
slippage_limit = 0.35
run_log = ["loaded orders", f"slippage_limit={slippage_limit}"]
summary = {
    "order_count": len(orders),
    "total_qty": sum(item["qty"] for item in orders),
    "symbols": sorted({item["symbol"] for item in orders}),
}
for item in orders:
    print(
        "  order {id} {side} {qty} {symbol} limit={limit}".format(**item)
    )
print(f"  prices={prices}")
print(f"  max_order_notional={max_order_notional} slippage_limit={slippage_limit}")
print(f"  summary={summary}")

# Breakpoint 1: must be a standalone statement.
print(SEP)
print("[checkpoint #1] VM snapshot saved; process exits now")
print("  note: next run with --resume continues from the next line")
print(SEP)
rpc.checkpoint(CHECKPOINT_PATH)

# Re-import after resume so the next checkpoint works.
# import rustpython_checkpoint as rpc  # type: ignore

# Phase 2: derive alerts and billing info from restored state.
print("[2/3] resumed after checkpoint #1")
print("  action: simulate fills, compute slippage and notional, flag risk, append run log")
fills = []
risk_flags = []
for item in orders:
    mkt = prices[item["symbol"]]
    slip = round(abs(mkt - item["limit"]), 2)
    notional = round(mkt * item["qty"], 2)
    fills.append(
        {
            "id": item["id"],
            "symbol": item["symbol"],
            "side": item["side"],
            "qty": item["qty"],
            "fill": mkt,
            "slippage": slip,
            "notional": notional,
        }
    )
    if slip > slippage_limit or notional > max_order_notional:
        risk_flags.append(item["id"])
total_notional = round(sum(item["notional"] for item in fills), 2)
run_log.append(f"risk_flags={risk_flags}")
run_log.append(f"total_notional={total_notional}")
print("  fills:")
for item in fills:
    print(
        "    - {id} {symbol} {side} qty={qty} fill={fill} "
        "slip={slippage} notional={notional}".format(**item)
    )
print("  risk_flags:")
if risk_flags:
    for item_id in risk_flags:
        print(f"    - {item_id}")
else:
    print("    - none")
print(f"  total_notional={total_notional}")
print("  log:")
for entry in run_log:
    print(f"    - {entry}")

# Breakpoint 2: save state again and exit; next run continues below.
print(SEP)
print("[checkpoint #2] VM snapshot saved; process exits now")
print("  note: next run with --resume continues from the next line")
print(SEP)
rpc.checkpoint(CHECKPOINT_PATH)

# After resume, prepare cleanup utilities.
# import os

# Phase 3: produce a final report and clean up the checkpoint file.
print(SEP)
print("[3/3] resumed after checkpoint #2")
print("  action: settle trades, build ledger, emit final report, and cleanup snapshot")
ledger = [
    {
        "symbol": item["symbol"],
        "net_qty": item["qty"] if item["side"] == "BUY" else -item["qty"],
        "avg_price": item["fill"],
    }
    for item in fills
]
report = {
    "summary": summary,
    "risk_flags": risk_flags,
    "ledger": ledger,
    "total_notional": total_notional,
    "status": "ready",
}
run_log.append("report_ready")
print("  report:")
print(f"    summary={report['summary']}")
print(f"    risk_flags={report['risk_flags']}")
print("    ledger:")
for item in report["ledger"]:
    print(
        "      - {symbol} net_qty={net_qty} avg_price={avg_price}".format(**item)
    )
print(f"    total_notional={report['total_notional']}")
print(f"    status={report['status']}")
print("  log:")
for entry in run_log:
    print(f"    - {entry}")
print(SEP)

# Clean up the checkpoint file so the next run starts fresh.
if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
print("[done] checkpoint file removed; next run starts fresh")
