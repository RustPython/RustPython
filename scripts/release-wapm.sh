#!/bin/bash
set -e

cd "$(dirname "$(dirname "$0")")"

FEATURES_FOR_WAPM=(stdlib zlib)

export BUILDTIME_RUSTPYTHONPATH="/lib/rustpython"

cargo build --release --target wasm32-wasi --no-default-features --features="${FEATURES_FOR_WAPM[*]}"

wapm publish
