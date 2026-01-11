#!/bin/bash

usage() {
    cat >&2 <<EOF
Usage: $0 [OPTIONS]

Copy tests from CPython to RustPython.
Optionally copy untracked tests, and dynamic annotation of failures.

Options:
  -c/--cpython-path <path>   Path to the CPython source tree (older version)
  -r/--rpython-path <path>   Path to the RustPython source tree (newer version)
  -u/--copy-untracked        Copy untracked tests only
  -s/--check-skipped         Check existing skipped tests (must be run separate from updating the tests)
  -t/--timeout               Set a timeout for a test
  -a/--annotate              While copying tests, run them and annotate failures dynamically
  -j/--jobs                  How many libraries can be processed at a time
  -h/--help                  Show this help message and exit

Example Usage: 
$0 -c ~/cpython -r .
$0 -r . --check-skipped

** Notes:
    * When using the update skip functionality
        * Updating only looks for files with the format "test_*.py". Everything else (including __init__.py and __main__.py files are ignored)
        * Care needs to be taken when updating. All tests with the same name will be updated at the same time
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
num_jobs=5
libraries=()

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
        -s|--check-skipped)
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
        sem
        check_skip "$lib" &
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
        echo "$lib failing, annotating..."
        readarray -t failed_tests <<< "$(echo "$output" | awk '/^(FAIL:|ERROR:)/ {print $2}' | sort -u)"
        
        # If the test fails/errors, then expectedFailure it
        for test in "${failed_tests[@]}"
        do
            add_above_test $rlib_path $test "@unittest.expectedFailure # TODO: RUSTPYTHON" 
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
            break;
        fi
    done

    unset SKIP_BACKUP
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
        echo "Message is $message"
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
        readarray -t libraries <<< $(find ${cpython_path} -type f -printf "%P\n")
    fi
    update_tests "${libraries[@]}"
else
    echo "Checking Skips"

    if [[ ${#libraries[@]} -eq 0 ]]; then
        readarray -t libraries <<< $(find ${rpython_path} -iname "test_*.py" -type f -printf "%P\n")
    fi
    check_skips "${libraries[@]}"
fi
