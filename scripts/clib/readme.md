To use, open cmd and run clib_test.py with args through python
and check the result in clib_out.txt

The script assumes that the script is being run from RustPython/scripts/clib,
and that both RustPython and cpython project directories are located under a same parent directory, aka that they are siblings

If either of those assumptions are false, then you must provide a correct path when running clib_test.py

The script will try to test every component in clib_list.txt