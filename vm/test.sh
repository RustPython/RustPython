#!/usr/bin/env bash
# Usage: test.sh <tests/test_case.py> [--bytecode|--dis]
#   --bytecode: print the bytecode
#   --dis: run dis

set -e

MODE="run"

case "${2}" in
  "--bytecode" )
    MODE="view_bytecode"
    ;;
  "--dis" )
    MODE="run_dis"
    ;;
  * )
    ;;
esac

TESTCASE=$(basename ${1})
#TMP_FILE="test_${TESTCASE}.bytecode"
TMP_FILE="${1}.bytecode"

python compile_code.py "${1}" > "${TMP_FILE}"

echo "${MODE}"
case "${MODE}" in 
  "run" )
    cd RustPython 
    RUST_BACKTRACE=1 cargo run "../${TMP_FILE}"
    ;;
  "view_bytecode" )
    cat "${TMP_FILE}" | python -m json.tool
    ;;
  "run_dis" )
    python -m dis "${1}"
    ;;
  * )
    echo "Not a valid mode!"
    ;;
esac

