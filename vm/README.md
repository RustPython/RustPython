RustPython
==============

A Python interpreter written in Rust

# Installation

```
bash init_env.sh
```

# Run

```
./test.sh <path/to/file.py> # compile and run
./test.sh <path/to/file.py> --bytecode # print the bytecode in JSON
./test.sh <path/to/file.py> --dis # Run python -m dis
```

## Manual
Given a python file `test.py`

```
python compile_code.py test.py > test.bytecode

cd RustPython
cargo run ../test.bytecode 
```

# Testing & debugging

```
./test_all.sh # Run all tests under tests/
```

* If a test is expected to fail or raise exception, add `xfail_*` prefix to the filename.

## Logging

```
RUST_LOG=debug ./tests_all.sh
```

# TODOs
* Native types => Partial
* Control flow => if(v)
* assert => OK
* Structural types (list, tuple, object)
* Strings
* Function calls => Blocked by bytecode serializer
* Modules import
* Generators


# Goals
* Support all builtin functions
* Runs the [pybenchmark](https://pybenchmarks.org/) benchmark test
* Run famous/popular python modules (which?)

* Compatible with CPython 3.6

# Rust version
rustc 1.20.0-nightly

