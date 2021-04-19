#!/bin/bash -e

get_crate_name() {
  while [[ $# -gt 1 ]]; do
    case "$1" in
      --crate-name)
        echo "$2"
        return
        ;;
    esac
    shift
  done
}

case $(get_crate_name "$@") in
  rustpython_*|rustpython)
    EXTRA=(-Zprofile -Ccodegen-units=1 -Cinline-threshold=0 -Clink-dead-code -Coverflow-checks=off -Zpanic_abort_tests) # -Cpanic=abort
    ;;

  *) EXTRA=() ;;
esac

exec "$@" "${EXTRA[@]}"
