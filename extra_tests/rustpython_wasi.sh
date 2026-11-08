#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
exec wasmer run --volume "$repo_root" \
  --env "RUSTPYTHONPATH=$repo_root/Lib" \
  "$repo_root/target/wasm32-wasip1/wasm-release/rustpython.wasm" -- "$@"
