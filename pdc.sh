
# Pre Delivery Check - or PDC for short - runs all build and check steps.
# The intension is to run all checks before delivering (or raising a PR or commiting or what ever you want to use it for).
#
# PDC executes all of the configured actions in the given order and reports which of them failed, 
# without stopping in case of an fail. 
# The actions in the given version are the RustPython CI steps plus a debug build in advance. 
# So you can run all of the checks in advance with one command.
#
# If you want to customize it or adapt the checks, just add or remove it from/ to the ACTIONS


ACTIONS=("cargo build" "cargo build --release" "cargo fmt --all" "cargo clippy --all -- -Dwarnings" "cargo test --all" "cargo run --release -- -m test -v" "cd tests" "pytest" "cd ..")

# Usually, there should be no need to adapt the remaining file, when adding or removing actions.


RUSTPYTHONPATH=Lib
export RUSTPYTHONPATH

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
