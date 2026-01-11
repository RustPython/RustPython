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
  -a/--annotate              While copying tests, run them and annotate failures dynamically
  -h/--help                  Show this help message and exit

Example Usage: $0 -c ~/cpython -r .
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
        -a|--annotate)
            annotate=true
            shift
            ;;
        *)
            libraries+=("$1.py")
            shift
            ;;
    esac
done

cpython_path="$cpython_path/Lib/test"
rpython_path="$rpython_path/Lib/test"

update_tests() {
    if [[ ${#libraries[@]} -eq 0 ]]; then
        libraries=$(find ${cpython_path} -type f -printf "%P\n")
    fi

    for lib in "${libraries[@]}"
    do 
        update_test "$lib"
    done
}

update_test() {
    lib=$1
    cpython_file="$cpython_path/$lib"
    rpython_file="$rpython_path/$lib"

    filename=${lib##*/}
    basename=${filename%.py}

    if files_equal "$cpython_file" "$rpython_file"; then
        echo "No changes in $lib. Skipping..." 
        continue
    fi

    if [[ ! -f "$rpython_file" ]]; then
        echo "Test file $lib missing"
        if $copy_untracked; then
            echo "Copying $lib ..."
            cp "$cpython_file" "$rpython_file"
        fi
    else
        echo "Using lib_updater to update $lib"
        ./scripts/lib_updater.py --from $cpython_file --to $rpython_file -o $rpython_file
    fi


    output=$(rustpython $lib 2>&1)
    if $annotate; then
        while ! grep -q "Tests result: SUCCESS" <<< "$output"; do
            echo "$lib failing, annotating..."
            failed_tests=$(echo "$output" | grep '^FAIL: ' | awk '{print $2}' | sort -u)
            
            # If the test fails/errors, then expectedFailure it
            for test in "${failed_tests[@]}"
            do
                sed -i "s/^\([[:space:]]*\)def $test(/\1@unittest.expectedFailure # TODO: RUSTPYTHON\n\1def $test(/" "$rpython_file"
            done

            # If the test crashes/hangs, then skip it
            if grep -q "\.\.\.$" <<< "$output"; then
                crashing_test=$(echo "$output" | grep '\.\.\.$' | head -n 1 | awk '{print $1}')
                if grep -q "Timeout" <<< "$output"; then
                    message="; hanging"
                fi
                sed -i "s/^\([[:space:]]*\)def $crashing_test(/\1@unittest.skip('TODO: RUSTPYTHON$message')\n\1def $crashing_test(/" "$rpython_file"
            fi

            output=$(rustpython $lib 2>&1)
        done
    fi
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

update_tests