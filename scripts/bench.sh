for BENCH in "int" "nbody" "fannkuch" "scimark"; do
 for CPYTHON in "3.9" "3.10"; do
  CMD="python${CPYTHON} ${BENCH}.py -o reports/${BENCH}_cpython${CPYTHON}.pyperf"
  echo "${CMD}"
  ${CMD} 
  sleep 1 
 done
 for RUSTPYTHON in "3819" "main"; do
  CMD="./target/release/rustpython_${RUSTPYTHON} ${BENCH}.py -o reports/${BENCH}_rustpython_${RUSTPYTHON}.pyperf"
  echo "${CMD}"
  ${CMD}  
  sleep 1 
 done
done
