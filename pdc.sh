RUSTPYTHONPATH=Lib
export RUSTPYTHONPATH

ACTIONS=("cargo build" "cargo build --release" "cargo fmt --all" "cargo clippy --all -- -Dwarnings" "cargo test --all" "cargo run --release -- -m test -v" "cd tests" "pytest" "cd ..")
ACT_RES=0
FAILS=()

for act in "${ACTIONS[@]}"; do
	$act
	if ! [ $? -eq  0 ]; then
		ACT_RES=1
		FAILS+=("${act}")
	fi
done

echo 
echo
echo "*********************"
if ! [ $ACT_RES -eq 0 ]; then
	echo "PDC failed"
	for el in "${FAILS[@]}"; do
		echo "     Fail ${el}"
	done
	echo
else
	echo "PDC passed"
fi
