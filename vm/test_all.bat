::  win bat 


cd tests

for %%i in (*.py) do python ../compile_code.py %%i >../bytes/%%i.bytecode
cd ..

REM cd RustPython
REM for %%i in (../bytes/*.vbytecode) do echo %%i
REM cd ..
