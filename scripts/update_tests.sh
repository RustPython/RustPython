#!/bin/bash

usage() { echo "Usage: $0 [-c <cpython path>] [-r <rustpython path>] [-u <copy untracked test>]" 1>&2; exit 1; }


cpython_path=""
rpython_path=""
copy_untracked=false
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
        *)
            libraries+=("$1")
            shift
            ;;
    esac
done

cpython_path="$cpython_path/Lib/test"
rpython_path="$rpython_path/Lib/test"


if [[ ${#libraries[@]} -eq 0 ]]; then
    libraries=$(find ${cpython_path} -type f -printf "%P\n")
fi
echo "libraries is ${libraries}"

for lib in "${arr[@]}"
do 
    cpython_file="$cpython_path/$lib"
    rpython_file="$rpython_path/$lib"


    if [[ $files_equal $cpython_file $rpython_file -eq 0 ]]; then
        continue
    fi

    if [[ ! -f $cpython_file ]]; then
        if $copy_untracked then
            echo "Test file $lib missing. Copying..."
            cp "$cpython_file" "$rpython_file"
        fi
    else
        echo "Updating $lib..."
        ./scripts/lib_updater.py --from ${rpython_path}/Lib/test/$lib --to ${rpython_path}/Lib/test/$lib
    fi
    

    cargo run
done



files_equal() {
    file1=$1
    file2=$2
    cmp --silent $file1 $file2 && files_equal=0 || files_equal=1
    return $files_equal
}

