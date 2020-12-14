
# Pre Delivery Check - or PDC for short - runs all build and check steps.
# The intension is to run all checks before delivering (or raising a PR or commiting or what ever you want to use it for).
#
# PDC executes all of the configured actions in the given order and reports which of them failed, 
# without stopping in case of an fail. 
# The actions in the given version are the RustPython CI steps plus a debug build in advance. 
# So you can run all of the checks in advance with one command.
#
# If you want to customize it or adapt the checks, just add or remove it from/ to the ACTIONS

# Preparation steps run before the first action is executed
# Typically we clear here the python cache to prevent dirty
# test results from cached python programs.
PRE_GLOBAL=("clear_pycache")

# Postprocessing steps run after the last ACTION has finished
POST_GLOBAL=()

# Preprocessing steps run before each action is executed
PRE_ACT=()

# Postprocessing steps run after each action is executed
POST_ACT=()

ACTIONS=("cargo build" "cargo build --release" "cargo fmt --all" "cargo clippy --all -- -Dwarnings" "cargo test --all" "cargo run --release -- -m test -v" "cd extra_tests" "pytest" "cd ..")

# Usually, there should be no need to adapt the remaining file, when adding or removing actions.

# clears the python cache or RustPython
clear_pycache() { find . -name __pycache__ -type d -exec rm -r {} \; ; }

RUSTPYTHONPATH=Lib
export RUSTPYTHONPATH

ACT_RES=0
FAILS=()

for pre in "${PRE_GLOBAL[@]}"; do
    $pre
done

for act in "${ACTIONS[@]}"; do

    for pre in "${PRE_ACTION[@]}"; do
        $pre
    done

	$act
	if ! [ $? -eq  0 ]; then
		ACT_RES=1
		FAILS+=("${act}")
	fi

    for pst in "${POST_ACTION[@]}"; do
        $pst
    done
done

for pst in "${OST_GLOBAL[@]}"; do
    $pst
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
