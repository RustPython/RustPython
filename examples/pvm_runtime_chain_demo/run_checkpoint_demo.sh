#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
demo_dir="$repo_root/examples/pvm_runtime_chain_demo"
bin="$repo_root/target/debug/examples/pvm_runtime_chain_demo"
state_dir="$repo_root/tmp/pvm_state"

cd "$repo_root"

lib_paths=()
if [ -d "/opt/homebrew/opt/libffi/lib" ]; then
  lib_paths+=("/opt/homebrew/opt/libffi/lib")
fi
if [ -d "/opt/homebrew/opt/libiconv/lib" ]; then
  lib_paths+=("/opt/homebrew/opt/libiconv/lib")
fi
dyld_prefix=""
if [ "${#lib_paths[@]}" -gt 0 ]; then
  dyld_prefix="$(IFS=:; echo "${lib_paths[*]}")"
fi

mkdir -p "$repo_root/tmp"
ts=$(date +%s)
if [ -e "$repo_root/tmp/pvm_state" ]; then
  mv "$repo_root/tmp/pvm_state" "$repo_root/tmp/pvm_state.bak.$ts"
fi
if [ -e "$repo_root/tmp/pvm_events.log" ]; then
  mv "$repo_root/tmp/pvm_events.log" "$repo_root/tmp/pvm_events.log.bak.$ts"
fi

cargo build --example pvm_runtime_chain_demo

if [ -n "$dyld_prefix" ]; then
  DYLD_LIBRARY_PATH="${dyld_prefix}${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
    "$bin" --continuation checkpoint --checkpoint-key 636865636b706f696e74 "$demo_dir/checkpoint_demo.py"
else
  "$bin" --continuation checkpoint --checkpoint-key 636865636b706f696e74 "$demo_dir/checkpoint_demo.py"
fi

export PVM_STATE_DIR="$state_dir"
python - <<'PY'
import json
import os
from pathlib import Path

state_dir = Path(os.environ["PVM_STATE_DIR"])
cid = (state_dir / "636964").read_bytes()
key = b"__runner_result:" + cid
payload = {"result": "ok"}
raw = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("ascii")
(state_dir / key.hex()).write_bytes(raw)
PY

if [ -n "$dyld_prefix" ]; then
  DYLD_LIBRARY_PATH="${dyld_prefix}${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
    "$bin" --resume-key 636865636b706f696e74 "$demo_dir/checkpoint_demo.py"
else
  "$bin" --resume-key 636865636b706f696e74 "$demo_dir/checkpoint_demo.py"
fi

python - <<'PY'
import os
from pathlib import Path

state_dir = Path(os.environ["PVM_STATE_DIR"])
step = (state_dir / "73746570").read_bytes()
result = (state_dir / "726573756c74").read_bytes()
print("step=", step)
print("result=", result)
PY
