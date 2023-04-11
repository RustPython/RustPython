#!/bin/bash

CPYTHONPATH=$(cat clib_path.txt)
RPYTHONPATH="../.."

if [ -f "clib_out.txt" ]; then
    rm "clib_out.txt"
fi

touch "clib_out.txt"

index=1

while :
do
    token=$(sed -n ${index}p clib_list.txt | xargs)
    if [ -z $token ] || [ ${token::1} == "#" ]; then
        ((index++))
        continue
    fi
    if [ $token == "EOF" ]; then
        break
    fi

    lib_exist=false
    test_exist=false
    test_do=true
    message="${token}:"

    if [ -f "${RPYTHONPATH}/Lib/${token}.py" ]; then
        lib_exist=true
        cp "${RPYTHONPATH}/Lib/${token}.py" "${RPYTHONPATH}/Lib/${token}_tmp_cp.py"
    fi
    if [ -f "${RPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        test_exist=true
        cp "${RPYTHONPATH}/Lib/test/test_${token}.py" "${RPYTHONPATH}/Lib/test/test_${token}_tmp_cp.py"
    fi

    if [ -f "${CPYTHONPATH}/Lib/${token}.py" ]; then
        cp "${CPYTHONPATH}/Lib/${token}.py" "${RPYTHONPATH}/Lib/${token}.py"
    fi
    if [ ! -f "${CPYTHONPATH}/Lib/${token}.py" ]; then
        test_do=false
        message="${message}  No cpython/Lib/${token}.py"
    fi
    
    if [ -f "${CPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        cp "${CPYTHONPATH}/Lib/test/test_${token}.py" "${RPYTHONPATH}/Lib/test/test_${token}.py"
    fi
    if [ ! -f "${CPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        test_do=false
        message="${message}  No cpython/Lib/test/test_${token}.py"
    fi

    if $test_do ; then
        test=$(cargo run -q "${RPYTHONPATH}/Lib/test/test_${token}.py" -q 2>&1 >/dev/null | grep "OK")

        if [ ! -z "${test}" ] ; then
            message="${message}  OK"
        fi
        if [ -z "${test}" ] ; then
            message="${message}  FAILED"
        fi
    fi

    if $lib_exist ; then
        mv "${RPYTHONPATH}/Lib/${token}_tmp_cp.py" "${RPYTHONPATH}/Lib/${token}.py"
    fi
    if ! $lib_exist && [ -f "${RPYTHONPATH}/Lib/${token}.py" ] ; then
        rm "${RPYTHONPATH}/Lib/${token}.py"
    fi 

     if $test_exist ; then
        mv "${RPYTHONPATH}/Lib/test/test_${token}_tmp_cp.py" "${RPYTHONPATH}/Lib/test/test_${token}.py"
    fi
    if ! $test_exist && [ -f "${RPYTHONPATH}/Lib/test/test_${token}.py" ] ; then
        rm "${RPYTHONPATH}/Lib/test/test_${token}.py"
    fi 

    echo $message >> "clib_out.txt"

    ((index++))
done