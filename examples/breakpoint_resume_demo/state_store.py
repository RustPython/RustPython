from __future__ import annotations

import json
from pathlib import Path
from typing import Any

BASE_DIR = Path(__file__).parent
DATA_DIR = BASE_DIR / "state"
STATE_FILE = DATA_DIR / "breakpoint_state.json"


def load_state() -> dict[str, Any] | None:
    # 读取断点状态文件；不存在则返回 None 表示首次运行
    if not STATE_FILE.exists():
        return None
    return json.loads(STATE_FILE.read_text())


def save_state(state: dict[str, Any]) -> None:
    # 写入断点状态，确保目录存在
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(json.dumps(state, sort_keys=True, indent=2))


def clear_state() -> None:
    # 清理断点状态，方便从头开始
    if STATE_FILE.exists():
        STATE_FILE.unlink()
