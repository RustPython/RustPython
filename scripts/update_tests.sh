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
  -h/--help                  Show this help message and exit

Example Usage: 
$0 -c ~/cpython -r .
$0 -r . --check-skipped
EOF
    exit 1
}

if [[ $# -eq 0 ]]; then
    usage
    exit 1
fi


cpython_path=""
rpython_path=""
copy_untracked=false
annotate=false
timeout=300
check_skip_flag=false
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
            return
            ;;
        -s|--check-skipped)
            check_skip_flag=true
            shift
            ;;
        -t|--timeout)
            timout="$2"
            shift 2
            ;;
        -a|--annotate)
            annotate=true
            shift
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
    libraries=$1
    for lib in "${libraries[@]}"
    do 
        update_test "$lib"
    done
}

update_test() {
    lib=$1
    clib_path="$cpython_path/$lib"
    rlib_path="$rpython_path/$lib"

    if files_equal "$clib_path" "$rlib_path"; then
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
    libraries=$1
    for lib in "${libraries[@]}"
    do
        check_skip "$lib"
    done
}

check_skip() {
    lib=$1
    rlib_path="$rpython_path/$lib"

    remove_skips $rlib_path

    annotate_lib $lib $rlib_path
}

annotate_lib() {
    lib=$(echo "$1" | sed 's/\//./g')
    rlib_path=$2
    output=$(rustpython $lib 2>&1)

    if grep -q "NO TESTS RAN" <<< "$output"; then
        echo "No tests ran in $lib. skipping annotation"
        return
    fi
    
    echo "Annotating $lib"

    while ! grep -q "Tests result: SUCCESS" <<< "$output"; do
        echo "$lib failing, annotating..."
        readarray -t failed_tests <<< $(echo "$output" | awk '/^(FAIL:|ERROR:)/ {print $2}' | sort -u)
        
        # If the test fails/errors, then expectedFailure it
        for test in "${failed_tests[@]}"
        do
            add_above_test $rlib_path $test "@unittest.expectedFailure # TODO: RUSTPYTHON" 
        done

        # If the test crashes/hangs, then skip it
        if grep -q "\.\.\.$" <<< "$output"; then
            crashing_test=$(echo "$output" | grep '\.\.\.$' | head -n 1 | awk '{print $1}')
            if grep -q "Timeout" <<< "$output"; then
                message="; hanging"
            fi
            add_above_test $rlib_path $crashing_test "@unittest.skip('TODO: RUSTPYTHON$message')"
        fi

        output=$(rustpython $lib 2>&1)
    done
}

files_equal() {
    file1=$1
    file2=$2
    cmp --silent $file1 $file2 && files_equal=0 || files_equal=1
    return $files_equal
}

rustpython() {
    cargo run --release --features encodings,sqlite -- -m test -j 1 -u all --fail-env-changed --timeout 300 -v "$@"
}

add_above_test() {
    file=$1
    test=$2
    line=$3
    sed -i "s/^\([[:space:]]*\)def $test(/\1$line\n\1def $test(/" "$file"
}

remove_skips() {
    rlib_path=$1

    echo "Removing all skips from $rlib_path"

    sed -i -E '/^[[:space:]]*@unittest\.skip.*\(["'\'']TODO\s?:\s?RUSTPYTHON.*["'\'']\)/Id' $rlib_path
}

if ! $check_skip_flag; then
    echo "Updating Tests"

    if [[ ${#libraries[@]} -eq 0 ]]; then
        readarray -t libraries <<< $(find ${cpython_path} -type f -printf "%P\n")
    fi
    update_tests $libraries
else
    echo "Checking Skips"

    readarray -t libraries <<< $(find ${rpython_path} -iname "test_*.py" -type f -printf "%P\n")
    check_skips $libraries
fi
