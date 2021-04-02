# RustPython Development Guide and Tips

RustPython attracts developers with interest and experience in Rust, Python,
or WebAssembly. Whether you are familiar with Rust, Python, or 
WebAssembly, the goal of this Development Guide is to give you the basics to
get set up for developing RustPython and contributing to this project. 

The contents of the Development Guide include:

- [Setting up a development environment](#setting-up-a-development-environment)
- [Code style](#code-style)
- [Testing](#testing)
- [Profiling](#profiling)
- [Code organization](#code-organization)
- [Understanding internals](#understanding-internals)
- [Questions](#questions)

## Setting up a development environment

RustPython requires the following:

- Rust latest stable version (e.g 1.51.0 as of Apr 2 2021)
    - To check Rust version: `rustc --version` 
    - If you have `rustup` on your system, enter to update to the latest
      stable version: `rustup update stable`
    - If you do not have Rust installed, use [rustup](https://rustup.rs/) to
      do so.
- CPython version 3.8 or higher
    - CPython can be installed by your operating system's package manager,
      from the [Python website](https://www.python.org/downloads/), or
      using a third-party distribution, such as 
      [Anaconda](https://www.anaconda.com/distribution/).
- [macOS] In case of libffi-sys compilation error, make sure autoconf, automake,
   libtool are installed
    - To install with [Homebrew](https://brew.sh), enter 
      `brew install autoconf automake libtool`
- [Optional] The Python package, `pytest`, is used for testing Python code
  snippets. To install, enter `python3 -m pip install pytest`.

## Code style

The Rust code style used is the default
[rustfmt](https://github.com/rust-lang/rustfmt) codestyle. Please format your
code accordingly, or run `cargo fmt` to autoformat it. We also use
[clippy](https://github.com/rust-lang/rust-clippy) to lint Rust code, which
you can check yourself with `cargo clippy`.

Custom Python code (i.e. code not copied from CPython's standard library) should
follow the [PEP 8](https://www.python.org/dev/peps/pep-0008/) style. We also use
[flake8](http://flake8.pycqa.org/en/latest/) to check Python code style.

## Testing

To test RustPython's functionality, a collection of Python snippets is located
in the `extra_tests/snippets` directory and can be run using `pytest`:

```shell
$ cd extra_tests
$ pytest -v
```

Rust unit tests can be run with `cargo`:

```shell
$ cargo test --all
```

## Profiling

To profile RustPython, build it in `release` mode with the `flame-it` feature.
This will generate a file `flamescope.json`, which can be viewed at
https://speedscope.app.

```shell
$ cargo run --release --features flame-it script.py
$ cat flamescope.json
{<json>}
```

You can specify another file name other than the default by using the
`--output-file` option to specify a file name (or `stdout` if you specify `-`).
The `--output-format` option determines the format of the output file.
The speedscope json format (default), text, or raw html can be passed. There
exists a raw html viewer which is currently broken, and we welcome a PR to fix it.

## Code organization

Understanding a new codebase takes time. Here's a brief view of the
repository's structure:

- `bytecode/src`: python bytecode representation in rust structures
- `compiler/src`: python compilation to bytecode
- `derive/src`: Rust language extensions and macros specific to rustpython
- `parser/src`: python lexing, parsing and ast
- `Lib`: Carefully selected / copied files from CPython sourcecode. This is
   the python side of the standard library.
  - `test`: CPython test suite
- `vm/src`: python virtual machine
  - `builtins`: Builtin functions and types
  - `stdlib`: Standard library parts implemented in rust.
- `src`: using the other subcrates to bring rustpython to life.
- `wasm`: Binary crate and resources for WebAssembly build
- `extra_tests`: extra integration test snippets as a supplement to `Lib/test`

## Understanding Internals

The RustPython workspace includes the `rustpython` top-level crate. The `Cargo.toml`
file in the root of the repo provide configuration of the crate and the
implementation is found in the `src` directory (specifically, `src/lib.rs`).

The top-level `rustpython` binary depends on several lower-level crates including:

- `rustpython-parser` (implementation in `parser/src`)
- `rustpython-compiler` (implementation in `compiler/src`)
- `rustpython-vm` (implementation in `vm/src`)

Together, these crates provide the functions of a programming language and
enable a line of code to go through a series of steps:

- parse the line of source code into tokens
- determine if the tokens are valid syntax
- create an Abstract Syntax Tree (AST)
- compile the AST into bytecode
- execute the bytecode in the virtual machine (VM).

### rustpython-parser

This crate contains the lexer and parser to convert a line of code to
an Abstract Syntax Tree (AST):

- Lexer: `parser/src/lexer.rs` converts Python source code into tokens
- Parser: `parser/src/parser.rs` takes the tokens generated by the lexer and parses
  the tokens into an AST (Abstract Syntax Tree) where the nodes of the syntax
  tree are Rust structs and enums.
  - The Parser relies on `LALRPOP`, a Rust parser generator framework. The
    LALRPOP definition of Python's grammar is in `parser/src/python.lalrpop`.
  - More information on parsers and a tutorial can be found in the 
    [LALRPOP book](https://lalrpop.github.io/lalrpop/README.html).
- AST: `ast/` implements in Rust the Python types and expressions
  represented by the AST nodes.

### rustpython-compiler

The `rustpython-compiler` crate's purpose is to transform the AST (Abstract Syntax
Tree) to bytecode. The implementation of the compiler is found in the
`compiler/src` directory. The compiler implements Python's symbol table,
ast->bytecode compiler, and bytecode optimizer in Rust.

Implementation of bytecode structure in Rust is found in the `bytecode/src`
directory. `bytecode/src/lib.rs` contains the representation of
instructions and operations in Rust. Further information about Python's
bytecode instructions can be found in the
[Python documentation](https://docs.python.org/3/library/dis.html#bytecodes).

### rustpython-vm

The `rustpython-vm` crate has the important job of running the virtual machine that
executes Python's instructions. The `vm/src` directory contains code to
implement the read and evaluation loop that fetches and dispatches
instructions. This directory also contains the implementation of the
Python Standard Library modules in Rust (`vm/src/stdlib`). In Python
everything can be represented as an object. The `vm/src/builtins` directory holds
the Rust code used to represent different Python objects and their methods. The
core implementation of what a Python object is can be found in
`vm/src/pyobjectrc.rs`.

## Questions

Have you tried these steps and have a question, please chat with us on
[gitter](https://gitter.im/rustpython/Lobby).
