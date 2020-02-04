#!/bin/bash

set -e

cargo=${1:-Cargo.toml}

tmpfile=$(mktemp)

awk '
/REDOX START/{redox=1; print; next}
/REDOX END/{redox=0}
{if (redox) print "#", $0; else print}
' "$cargo" >"$tmpfile"

mv "$tmpfile" "$cargo"
