#!/bin/bash

CPYTHONPATH=$(cat clib_path.txt)

if [ -f "clib_out.txt" ]; then
    rm "clib_out.txt"
fi

touch "clib_out.txt"

index=1

while :
do
    token=$(cat clib_list.txt | tr '\n' ' ' | tr '  ' ' ' | tr '  ' ' ' | cut -d ' ' -f $index)
    if [ -z $token ] || [ $token == "#" ]; then
        break
    fi

    lib_exist=false
    test_exist=false
    test_do=true
    message="${token}:"

    if [ -f "Lib/${token}.py" ]; then
        lib_exist=true
        cp "Lib/${token}.py" "Lib/${token}_tmp_cp.py"
    fi
    if [ -f "Lib/test/test_${token}.py" ]; then
        test_exist=true
        cp "Lib/test/test_${token}.py" "Lib/test/test_${token}_tmp_cp.py"
    fi

    if [ -f "${CPYTHONPATH}/Lib/${token}.py" ]; then
        cp "${CPYTHONPATH}/Lib/${token}.py" "Lib/${token}.py"
    fi
    if [ ! -f "${CPYTHONPATH}/Lib/${token}.py" ]; then
        test_do=false
        message="${message}  No cpython/Lib/${token}.py"
    fi
    
    if [ -f "${CPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        cp "${CPYTHONPATH}/Lib/test/test_${token}.py" "Lib/test/test_${token}.py"
    fi
    if [ ! -f "${CPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        test_do=false
        message="${message}  No cpython/Lib/test/test_${token}.py"
    fi

    if $test_do ; then
        test=$(cargo run -q "Lib/test/test_${token}.py" -q 2>&1 >/dev/null | grep "OK")

        if [ ! -z "${test}" ] ; then
            message="${message}  OK"
        fi
        if [ -z "${test}" ] ; then
            message="${message}  FAILED"
        fi
    fi

    if $lib_exist ; then
        mv "Lib/${token}_tmp_cp.py" "Lib/${token}.py"
    fi
    if ! $lib_exist && [ -f "Lib/${token}.py" ] ; then
        rm "Lib/${token}.py"
    fi 

     if $test_exist ; then
        mv "Lib/test/test_${token}_tmp_cp.py" "Lib/test/test_${token}.py"
    fi
    if ! $test_exist && [ -f "Lib/test/test_${token}.py" ] ; then
        rm "Lib/test/test_${token}.py"
    fi 

    echo $message >> "clib_out.txt"

    ((index++))
done