from __future__ import annotations

import json
from pathlib import Path
from typing import Any

BASE_DIR = Path(__file__).parent
DATA_DIR = BASE_DIR / "state"
STATE_FILE = DATA_DIR / "breakpoint_state.json"


def load_state() -> dict[str, Any] | None:
    # Load checkpoint state file; return None when missing to indicate first run.
    if not STATE_FILE.exists():
        return None
    return json.loads(STATE_FILE.read_text())


def save_state(state: dict[str, Any]) -> None:
    # Write checkpoint state, ensuring the directory exists.
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(json.dumps(state, sort_keys=True, indent=2))


def clear_state() -> None:
    # Clear checkpoint state to allow a fresh start.
    if STATE_FILE.exists():
        STATE_FILE.unlink()
