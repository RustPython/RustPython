#!/usr/bin/env bash
# set -e 

source venv/bin/activate
#TESTCASE=tests/variables.py
# TESTCASE=tests/variables.py
#TESTCASE=tests/minimum.py
unexpected_count=0
expected_count=0
fail_titles=$""

RED='\033[0;31m'
NC='\033[0m' # No Color

for TESTCASE in $(find tests -name \*.py -print)
do
  echo "TEST START: ${TESTCASE}"
  echo "--------------------------------"
  FILENAME="$(basename ${TESTCASE})"
  xfail=false
  if [ "${FILENAME:0:6}" = "xfail_" ]; then
    echo "Expected FAIL"
    xfail=true
  fi


  python compile_code.py $TESTCASE > $TESTCASE.bytecode
  cd RustPython
  cargo run ../$TESTCASE.bytecode


  if [[ $? -ne 0 ]]; then
    if [ "${xfail}" = true ]; then
      echo "== FAIL as expected  =="
      let expected_count=expected_count+1
    else
      printf "${RED}== FAIL, expected PASS ==${NC}\n"
      let unexpected_count=unexpected_count+1
      fail_titles=$"${fail_titles}\n${TESTCASE}\t${RED}FAIL${NC} (expected PASS)"
    fi
  else
    if [ "${xfail}" = true ]; then
      printf "${RED}== PASS, expected FAIL ==${NC}\n"
      let unexpected_count=unexpected_count+1
      let unexpected_count=unexpected_count+1
      fail_titles=$"${fail_titles}\n${TESTCASE}\t${RED}PASS${NC} (expected FAIL)"
    else
      echo "== PASS as expected  =="
      let expected_count=expected_count+1
    fi
  fi
  cd ..
  echo "--------------------------------"

done

echo "Summary"
echo "================"
printf "${RED}${unexpected_count} unexpected${NC}, ${expected_count} expected"
echo ""
echo ""
echo "unexpected results:"
printf "${fail_titles}"
echo ""
echo "================"
