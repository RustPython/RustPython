#!/bin/sh
set -e

cd "$(dirname "$0")"
cd tests

python3 not_impl_gen.py

cd ..

cargo run -- tests/snippets/whats_left_to_implement.py