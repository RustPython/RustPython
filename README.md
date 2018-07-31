# RustPython
A Python Interpreter written in Rust :snake: :scream: :metal:.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)

# Usage (Not implemented yet)

To test RustPython, do the following:

    $ git clone https://github.com/RustPython/RustPython
    $ cd RustPython
    $ cargo run demo.py
    42

Or use the interactive shell:

    $ cargo run
    Welcome to rustpython
    >>>>> 2+2
    4

Or use pip to install extra modules:

    $ cargo run -m pip install requests

# Goals

- Full python environment entirely in Rust (not CPython bindings)
- A clean implementation without compatibility hacks

# Code organization

    - `parser`: python lexing, parsing and ast
    - `vm`: python virtual machine
    - `src`: using the other subcrates to bring rustpython to life.
    - `docs`: documentation (work in progress)
    - `py_code_object`: CPython bytecode to rustpython bytecode convertor (work in progress)
    - `tests`: integration test snippets

# Community

Chat with us on [gitter][gitter].

# Credit

The initial work was based on [windelbouwman/rspython](https://github.com/windelbouwman/rspython) and [shinglyu/RustPython](https://github.com/shinglyu/RustPython)

[gitter]: https://gitter.im/rustpython/Lobby

# Links

These are some useful links to related projects:

- https://github.com/ProgVal/pythonvm-rust
- https://github.com/shinglyu/RustPython
- https://github.com/windelbouwman/rspython

