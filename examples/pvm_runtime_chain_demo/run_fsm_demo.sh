#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
demo_dir="$repo_root/examples/pvm_runtime_chain_demo"
bin="$repo_root/target/debug/examples/pvm_runtime_chain_demo"

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
    "$bin" --continuation fsm "$demo_dir/fsm_demo.py" start
  DYLD_LIBRARY_PATH="${dyld_prefix}${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
    "$bin" --continuation fsm "$demo_dir/fsm_demo.py" ok
else
  "$bin" --continuation fsm "$demo_dir/fsm_demo.py" start
  "$bin" --continuation fsm "$demo_dir/fsm_demo.py" ok
fi
