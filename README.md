# RustPython
A Python Interpreter written in Rust

# Code organization
The files in the top level directory are from [windelbouwman/rspython][rspython] which contains an implementation of the parser and vm in `src/`

An alternative implementation of python virtual machine that are compatible with CPython parser are from [shinglyu/RustPython][rustpython] and is located in the `VM/` folder.

We are in the process of merging the two implementation to form a single implementation.

# Credit
The initial work was based on [windelbouwman/rspython](https://github.com/windelbouwman/rspython) and [shinglyu/RustPython](https://github.com/shinglyu/RustPython)

[rspython]: https://github.com/windelbouwman/rspython
[rustpython]: https://github.com/shinglyu/RustPython

