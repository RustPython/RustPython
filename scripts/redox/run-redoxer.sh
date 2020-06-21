#!/usr/bin/env bash

set -eo pipefail

cd "$(dirname "$0")"
cd ../..

DIR=$(mktemp -d)

trap 'cd / && rm -rf "$DIR"' EXIT SIGINT

BUILDTIME_RUSTPYTHONPATH=/root/rustpython-lib redoxer build --release

cp target/x86_64-unknown-redox/release/rustpython -t "$DIR"
ln -s "$PWD"/Lib "$DIR"/rustpython-lib

redoxer exec -f "$DIR" -- ./rustpython "$@"
