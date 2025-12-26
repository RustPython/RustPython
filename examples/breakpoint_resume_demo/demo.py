from __future__ import annotations

from pathlib import Path

import rustpython_checkpoint as rpc  # type: ignore

# Checkpoint file path as a string to keep it serializable.
CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))

# Phase 1: prepare variables and context.
print("[run] phase=init")
user = "Tony"
amount = 120
items = [f"item_{idx}" for idx in range(3)]
analysis = {"score": 0.6, "summary": "score=0.6"}
imhere = "Tony, I'm here \n"
print(f"[run] user={user} amount={amount} items={items} analysis={analysis}")
print(f"IMHERE:\n{imhere}")


# Breakpoint 1: must be a standalone statement, not inside assignments/conditions.
# When reaching this line, RustPython saves the VM state and exits the process.
rpc.checkpoint(CHECKPOINT_PATH)

# Re-import after resume so the next checkpoint works.
import rustpython_checkpoint as rpc  # type: ignore

# Phase 2: continue using variables restored from the previous run.
print("[run] phase=after_checkpoint_1")
processed = [f"{user}:{item}" for item in items]
total = amount + len(processed)
imhere += "Yusuf, I'm here \n"
print(f"[run] processed={processed} total={total}")
print(f"IMHERE:\n{imhere}")
# Breakpoint 2: save state again and exit; next run continues below.
rpc.checkpoint(CHECKPOINT_PATH)




# After resume, prepare cleanup utilities.
import os

# Phase 3: execute after resuming from the second breakpoint.
print("[run] phase=after_checkpoint_2")
receipt = {
    "user": user,
    "total": total,
    "processed": processed,
    "status": "ok",
}
imhere += "Zeta, Johny, We're here \n"
print(f"[run] receipt={receipt}")
print(f"IMHERE:\n{imhere}")

# Clean up the checkpoint file so the next run starts fresh.
if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
print("[run] done")
