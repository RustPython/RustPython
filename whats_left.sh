#!/bin/sh

cd "$(dirname "$0")" || exit

cargo run -- tests/snippets/whats_left_to_implement.py
