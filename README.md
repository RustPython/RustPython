# RustPython
A Python-3  (CPython >= 3.5.0) Interpreter written in Rust :snake: :scream: :metal:.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

# Usage

To test RustPython, do the following:

    $ git clone https://github.com/RustPython/RustPython
    $ cd RustPython
    $ cargo run demo.py
    Hello, RustPython!

Or use the interactive shell:

    $ cargo run
    Welcome to rustpython
    >>> 2+2
    4

<!-- Or use pip to install extra modules:

    $ cargo run -m pip install requests -->

# Goals

- Full Python-3 environment entirely in Rust (not CPython bindings)
- A clean implementation without compatibility hacks

# Code organization

- `parser`: python lexing, parsing and ast
- `vm`: python virtual machine
- `src`: using the other subcrates to bring rustpython to life.
- `docs`: documentation (work in progress)
- `py_code_object`: CPython bytecode to rustpython bytecode convertor (work in progress)
- `tests`: integration test snippets

# Contributing

To start contributing, there are a lot of things that need to be done.
Most tasks are listed in the [issue tracker](https://github.com/RustPython/RustPython/issues).
Another approach is to checkout the sourcecode, and try out rustpython until
you hit a limitation, and try to fix that.

# Testing

To test rustpython, there is a collection of python snippets located in the
`tests/snippets` directory. To run those tests do the following:

```shell
$ cd tests
$ pipenv shell
$ pytest -v
```

There also are some unittests, you can run those will cargo:

```shell
$ cargo test --all
```

# Code style

The code style used is the default rustfmt codestyle. Please format your code
accordingly.

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

