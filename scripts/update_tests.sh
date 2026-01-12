#!/bin/bash

usage() {
    cat >&2 <<EOF
Usage: $0 [OPTIONS]

Copy tests from CPython to RustPython.
Optionally copy untracked tests, and dynamic annotation of failures.
This updater is not meant to be used as a standalone updater. Care needs to be taken to review everything done

Options:
  -c/--cpython-path <path>   Path to the CPython source tree (older version)
  -r/--rpython-path <path>   Path to the RustPython source tree (newer version)
  -s/--update-skipped        Update existing skipped tests (must be run separate from updating the tests)
  -a/--annotate              While copying tests, run them and annotate failures dynamically
  -u/--copy-untracked        Copy untracked tests
  -t/--timeout               Set a timeout for a test
  -j/--jobs                  How many libraries can be processed at a time
  -h/--help                  Show this help message and exit

Example Usage: 
$0 -c ~/cpython -r - . -t 300   # Updates all non-updated tests with a timeout value of 300 seconds
$0 -c ~/cpython -r . -u -j 5    # Updates all non-updated tests + copies files not in cpython into rpython, with maximum 5 processes active at a time
$0 -c ~/cpython -r . -a         # Updates all non-updated tests + annotates with @unittest.expectedFailure/@unittest.skip
$0 -r . -s                      # For all current tests, check if @unittest.skip can be downgraded to @unittest.expectedFailure

*Notes:
    * When using the update skip functionality
        * Updating only looks for files with the format "test_*.py". Everything else (including __init__.py and __main__.py files are ignored)
**Known limitations:
    * In multithreaded tests, if the tests are orphaned, then the updater can deadlock, as threads can accumulate and block the semaphore
    * The updater does not add skips to classes, only on tests
    * If there are multiple tests with the same name, a decorator will be added to all of them
    * The updater does not retain anything more specific than a general skip (skipIf/Unless will be replaced by a general skip)
    * Currently, the updater does not take unexpected successes into account

EOF
    exit 1
}

if [[ $# -eq 0 ]]; then
    usage 
fi


cpython_path=""
rpython_path=""
copy_untracked=false
annotate=false
timeout=300
check_skip_flag=false
num_jobs=20
libraries=()
ignored_libraries=("multiprocessing" "concurrent")

while [[ $# -gt 0 ]]; do
    case "$1" in
        -c|--cpython-path)
            cpython_path="$2"
            shift 2
            ;;
        -r|--rpython-path)
            rpython_path="$2"
            shift 2
            ;;
        -u|--copy-untracked)
            copy_untracked=true
            shift
            ;;
        -h|--help)
            usage
            exit 1
            ;;
        -s|--update-skipped)
            check_skip_flag=true
            shift
            ;;
        -t|--timeout)
            timeout="$2"
            shift 2
            ;;
        -a|--annotate)
            annotate=true
            shift
            ;;
        -j|--jobs)
            num_jobs="$2"
            shift 2
            ;;
        *)
            libraries+=("$1")
            shift
            ;;
    esac
done

cpython_path="$cpython_path/Lib/test"
rpython_path="$rpython_path/Lib/test"


update_tests() {
    local libraries=("$@")
    for lib in "${libraries[@]}"
    do 
        sem
        update_test "$lib" &
    done
    wait
}

update_test() {
    local lib=$1
    local clib_path="$cpython_path/$lib"
    local rlib_path="$rpython_path/$lib"

    if [[ -f "$clib_path" && -f "$rlib_path" ]] && files_equal "$clib_path" "$rlib_path"; then
        echo "No changes in $lib. Skipping..." 
        return
    fi

    if [[ ! -f "$rlib_path" ]]; then
        echo "Test file $lib missing"
        if $copy_untracked; then
            echo "Copying $lib ..."
            cp "$clib_path" "$rlib_path"
        fi
    else
        echo "Using lib_updater to update $lib"
        ./scripts/lib_updater.py --from $clib_path --to $rlib_path -o $rlib_path
    fi


    if [[ $annotate && -f "$rlib_path" && $(basename -- "$rlib_path") == test_*.py ]]; then
        annotate_lib $lib $rlib_path
    fi
}

check_skips() {
    local libraries=("$@")
    for lib in "${libraries[@]}"
    do
        if grep -qiE "@unittest.skip.*\('TODO:\s*RUSTPYTHON.*'\)" "$rpython_path/$lib"; then
            sem
            check_skip "$lib" &
        else
            echo "Skipping $lib" >&2
        fi
    done
    wait
}

check_skip() {
    local lib=$1
    local rlib_path="$rpython_path/$lib"

    remove_skips $rlib_path

    annotate_lib $lib $rlib_path
}

annotate_lib() {
    local lib=${1//\//.}
    local rlib_path=$2
    local output=$(rustpython $lib 2>&1)

    if grep -q "NO TESTS RAN" <<< "$output"; then
        echo "No tests ran in $lib. skipping annotation"
        return
    fi
    
    echo "Annotating $lib"

    local attempts=0
    while ! grep -q "Tests result: SUCCESS" <<< "$output"; do
        ((attempts++))
        # echo "$lib failing, annotating..."
        readarray -t failed_tests <<< "$(echo "$output" | awk '/^(FAIL:|ERROR:)/ {print $2}' | sort -u)"
        
        # If the test fails/errors, then expectedFailure it
        for test in "${failed_tests[@]}"
        do
            if already_failed $rlib_path $test; then
                replace_expected_with_skip $rlib_path $test
            else
                add_above_test $rlib_path $test "@unittest.expectedFailure # TODO: RUSTPYTHON" 
            fi
        done

        # If the test crashes/hangs, then skip it
        if grep -q "\.\.\.$" <<< "$output"; then
            crashing_test=$(echo "$output" | grep '\.\.\.$' | head -n 1 | awk '{print $1}')
            if grep -q "Timeout" <<< "$output"; then
                hanging=true
            else
                hanging=false
            fi
            apply_skip "$rlib_path" "$crashing_test" $hanging
        fi

        output=$(rustpython $lib 2>&1)

        if [[ attempts -gt 10 ]]; then 
            echo "Issue annotating $lib" >&2
            return;
        fi
    done
    echo "Successfully updated $lib"

    unset SKIP_BACKUP
}

replace_expected_with_skip() {
    file=$1
    test_name=$2
    sed -E "/^\s*@unittest\.expectedFailure\s+# TODO: RUSTPYTHON/ { N; /\n\s*def $test_name/ { s/^(\s*)@unittest\.expectedFailure\s+# TODO: RUSTPYTHON/\1@unittest.skip\('TODO: RUSTPYTHON'\)/ } }" -i $file
}

already_failed() {
    file=$1
    test_name=$2
    grep -qPz "\s*@unittest\.expectedFailure # TODO: RUSTPYTHON\n\s*def\s+${test_name}\(" $file
}

files_equal() {
    cmp --silent "$1" "$2"
}

rustpython() {
    cargo run --release --features encodings,sqlite -- -m test -j 1 -u all --fail-env-changed --timeout "$timeout" -v "$@"
}

sem() {
    while (( $(jobs -rp | wc -l) >= $num_jobs )); do
        sleep 0.1  # brief pause before checking again
    done
}

add_above_test() {
    local file=$1
    local test=$2
    local line=$3
    sed -i "s/^\([[:space:]]*\)def $test(/\1$line\n\1def $test(/" "$file"
}

remove_skips() {
    local rlib_path=$1

    echo "Removing all skips from $rlib_path"

    backup_skips "$rlib_path"

    sed -i -E '/^[[:space:]]*@unittest\.skip.*\(["'\'']TODO\s?:\s?RUSTPYTHON.*["'\'']\)/Id' $rlib_path
}

apply_skip() {
    local rlib_path=$1
    local test_name=$2
    local hanging=$3
    message="unknown"

    # Check if the test has a backup skip
    if [[ -n "${SKIP_BACKUP[$test_name]}" ]]; then
        message="${SKIP_BACKUP[$test_name]//\'/\"}" 
    elif $hanging; then
        message="hanging"
    fi

    add_above_test "$rlib_path" "$test_name" "@unittest.skip('TODO: RUSTPYTHON; $message')"
}

backup_skips() {
    local rlib_path=$1
    declare -gA SKIP_BACKUP=()  # global associative array
    readarray -t skips < <(grep -E -n "^[[:space:]]*@unittest\.skip.*TODO\s?:\s?RUSTPYTHON" "$rlib_path" | sort -u)

    for line in "${skips[@]}"; do
        line_num="${line%%:*}"
        line_text=$(echo "$line" | grep -oPi "(?<=RUSTPYTHON)\s*[;:]\s*\K(.*)?(?=[\"'])")
        next_line=$(sed -n "$((line_num + 1))p" "$rlib_path")

        if [[ "$next_line" =~ def[[:space:]]+([a-zA-Z0-9_]+)\( ]]; then
            test_name="${BASH_REMATCH[1]}"
            SKIP_BACKUP[$test_name]="$line_text"
        fi
    done
}

if ! $check_skip_flag; then
    echo "Updating Tests"

    if [[ ${#libraries[@]} -eq 0 ]]; then
        readarray -t libraries <<< $(find ${cpython_path} -type f -printf "%P\n" | grep -vE "$(IFS=\|; echo "${ignored_libraries[*]}")")
    fi
    update_tests "${libraries[@]}"
else
    echo "Checking Skips"

    if [[ ${#libraries[@]} -eq 0 ]]; then
        readarray -t libraries <<< $(find ${rpython_path} -iname "test_*.py" -type f -printf "%P\n" | grep -vE "$(IFS=\|; echo "${ignored_libraries[*]}")")
    fi
    check_skips "${libraries[@]}"
fi
