#!/bin/bash
set -e

# This script runs a Python script which finds all modules it has available and
# creates a Python dictionary mapping module names to their contents, which is
# in turn used to generate a second Python script that also finds which modules
# it has available and compares that against the first dictionary we generated.
# We then run this second generated script with RustPython.

cd "$(dirname "$0")"

export RUSTPYTHONPATH=Lib

(
  cd extra_tests
  # -I means isolate from environment; we don't want any pip packages to be listed
  python3 -I not_impl_gen.py
)

# This takes a while
if command -v black &> /dev/null; then
    black -q extra_tests/snippets/not_impl.py
fi

# show the building first, so people aren't confused why it's taking so long to
# run whats_left
cargo build --release

cargo run --release -q -- extra_tests/snippets/not_impl.py
