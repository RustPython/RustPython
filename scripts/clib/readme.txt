To use, write down your local cpython path in clib_path.txt, and run,
bash clib_test_one.sh

and check the result in clib_out.txt

The script will try to test every component in clib_list.txt, where,

clib_test_one.sh : test each component individually
clib_test_all.sh : test every components simultaneously

Use of clib_test_all.sh is discouraged, as one failing component can lead to cascading failure of otherwise non-failing components