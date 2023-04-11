#!/bin/bash

CPYTHONPATH=$(cat clib_path.txt)
RPYTHONPATH="../.."

cp -r "${RPYTHONPATH}/Lib" "${RPYTHONPATH}/LibTmp"

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

    if [ -f "${CPYTHONPATH}/Lib/${token}.py" ]; then
        cp "${CPYTHONPATH}/Lib/${token}.py" "${RPYTHONPATH}/Lib/${token}.py"
    fi
    
    if [ -f "${CPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        cp "${CPYTHONPATH}/Lib/test/test_${token}.py" "${RPYTHONPATH}/Lib/test/test_${token}.py"
    fi

    ((index++))
done

cargo build -r

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

    message="${token}:"

    if [ -f "${RPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        test=$(cargo run -q -r "${RPYTHONPATH}/Lib/test/test_${token}.py" -q 2>&1 >/dev/null | grep "OK")

        if [ ! -z "${test}" ] ; then
            message="${message}  OK"
        fi
        if [ -z "${test}" ] ; then
            message="${message}  FAILED"
        fi
    fi
    if [ ! -f "${RPYTHONPATH}/Lib/test/test_${token}.py" ]; then
        message="${message}  NOTEST"
    fi

    echo $message >> "clib_out.txt"

    ((index++))
done

rm -r "${RPYTHONPATH}/Lib"
mv -r "${RPYTHONPATH}/LibTmp" "${RPYTHONPATH}/Lib"