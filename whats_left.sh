#!/bin/bash
set -e

ALL_SECTIONS=(methods modules)

h() {
  # uppercase input
  header_name=$(echo "$@")
  echo "$header_name: "
}

cd "$(dirname "$0")"

export RUSTPYTHONPATH=Lib

(
  cd tests
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
  snippet=tests/snippets/whats_left_$section.py
  if ! [[ -f $snippet ]]; then
    echo "Invalid section $section" >&2
    continue
  fi
  h "$section"
  results=$(cargo run --release -q -- "$snippet" ) 

  # (inherited) was showing up in the list as a separate line.
  # I manually excluded it from results.
  # is there a better way to do this or make sure inherited is not on a separate line?
  for result in $results; do
    if [[  "$result" != "(inherited)"  ]]; then
      printf " - $result \n"
    fi
  done

done
