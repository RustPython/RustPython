#!/bin/sh
set -e

GREEN='[32m'
BOLD='[1m'
NC='(B[m'

h() {
  # uppercase input
  header_name=$(echo "$@" | tr "[:lower:]" "[:upper:]")
  echo "$GREEN$BOLD===== $header_name =====$NC"
}

cd "$(dirname "$0")"

(
  cd tests
  # -I means isolate from environment; we don't want any pip packages to be listed
  python3 -I not_impl_gen.py
)

# show the building first, so people aren't confused why it's taking so long to
# run whats_left_to_implement
cargo build

whats_left_section() {
  h "$1"
  cargo run -q -- tests/snippets/whats_left_"$1".py
}

whats_left_section methods
whats_left_section modules
