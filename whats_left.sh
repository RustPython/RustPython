#!/bin/bash
set -e

ALL_SECTIONS=(methods modules)

GREEN='[32m'
BOLD='[1m'
NC='(B[m'

h() {
  # uppercase input
  header_name=$(echo "$@" | tr "[:lower:]" "[:upper:]")
  echo "$GREEN$BOLD===== $header_name =====$NC"
}

cd "$(dirname "$0")"

export RUSTPYTHONPATH=Lib

(
  cd extra_tests
  # -I means isolate from environment; we don't want any pip packages to be listed
  python3 -I not_impl_gen.py
)

# show the building first, so people aren't confused why it's taking so long to
# run whats_left_to_implement
cargo build --release

if [ $# -eq 0 ]; then
  sections=(${ALL_SECTIONS[@]})
else
  sections=($@)
fi

for section in "${sections[@]}"; do
  section=$(echo "$section" | tr "[:upper:]" "[:lower:]")
  snippet=extra_tests/snippets/whats_left_$section.py
  if ! [[ -f $snippet ]]; then
    echo "Invalid section $section" >&2
    continue
  fi
  h "$section" >&2
  cargo run --release -q -- "$snippet"
done
