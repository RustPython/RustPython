from __future__ import annotations

from pathlib import Path

import rustpython_checkpoint as rpc  # type: ignore

# Checkpoint file path as a string to keep it serializable.
CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))


def section(title: str) -> None:
    print("\n" + "=" * 60)
    print(title)
    print("=" * 60)


section("PVM Breakpoint/Resume Showcase")
print("[run] phase=init")
customer = "Acme Corp"
order_id = "ORD-2049"
items = [f"sku_{i:02d}" for i in range(3)]
score = 0.87
notes = ["session started", "items captured"]
print(f"[run] customer={customer} order_id={order_id}")
print(f"[run] items={items} score={score}")
print(f"[run] notes={notes}")

# Breakpoint 1: must be a standalone statement.
# RustPython saves VM state here and exits the process.
rpc.checkpoint(CHECKPOINT_PATH)

# Re-import after resume so the next checkpoint works.
import rustpython_checkpoint as rpc  # type: ignore

# Recreate helpers after resume (functions are not checkpoint-serializable).
def section(title: str) -> None:
    print("\n" + "=" * 60)
    print(title)
    print("=" * 60)

section("Resume #1: state restored")
print("[run] phase=after_checkpoint_1")
priced = [f"{item}:$99" for item in items]
total = 99 * len(priced)
notes.append("pricing complete")
print(f"[run] priced={priced}")
print(f"[run] total={total} notes={notes}")

# Breakpoint 2: save state again and exit; next run continues below.
rpc.checkpoint(CHECKPOINT_PATH)

import os

# Recreate helpers after resume (functions are not checkpoint-serializable).
def section(title: str) -> None:
    print("\n" + "=" * 60)
    print(title)
    print("=" * 60)

section("Resume #2: finishing up")
print("[run] phase=after_checkpoint_2")
receipt = {
    "customer": customer,
    "order_id": order_id,
    "total": total,
    "status": "ok",
}
notes.append("receipt issued")
print(f"[run] receipt={receipt}")
print(f"[run] notes={notes}")

# Clean up so a fresh run starts from the top.
if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
print("[run] done")
