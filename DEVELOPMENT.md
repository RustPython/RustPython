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

- Rust latest stable version (e.g 1.92.0 as of Jan 7 2026)
    - To check Rust version: `rustc --version` 
    - If you have `rustup` on your system, enter to update to the latest
      stable version: `rustup update stable`
    - If you do not have Rust installed, use [rustup](https://rustup.rs/) to
      do so.
- CPython version 3.14 or higher
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
[ruff](https://beta.ruff.rs/docs/) to check Python code style.

In addition to language specific tools, [cspell](https://github.com/streetsidesoftware/cspell),
a code spell checker, is used in order to ensure correct spellings for code.

## Testing

To test RustPython's functionality, a collection of Python snippets is located
in the `extra_tests/snippets` directory and can be run using `pytest`:

```shell
$ cd extra_tests
$ pytest -v
```

Rust unit tests can be run with `cargo`:

```shell
$ cargo test --workspace --exclude rustpython_wasm
```

Python unit tests can be run by compiling RustPython and running the test module:

```shell
$ cargo run --release -- -m test
```

There are a few test options that are especially useful:

- `-j <n>` enables parallel testing (which is a lot faster), where `<n>` is the
number of threads to be used, ideally the same as number of cores on your CPU.
If you don't know, `-j 4` or `-j 8` are good options.
- `-v` enables verbose mode, adding additional information about the tests being
run.
- `<test_name>` specifies a single test to run instead of running all tests.

For example, to run all tests in parallel:

```shell
$ cargo run --release -- -m test -j 4
```

To run only `test_cmath` (located at `Lib/test/test_cmath`) verbosely:

```shell
$ cargo run --release -- -m test test_cmath -v
```

### Testing on Linux from macOS

You can test RustPython on Linux from macOS using Apple's `container` CLI.

**Setup (one-time):**

```shell
# Install container CLI
$ brew install container

# Disable Rosetta requirement for arm64-only builds
$ defaults write com.apple.container.defaults build.rosetta -bool false

# Build the development image
$ container build --arch arm64 -t rustpython-dev -f .devcontainer/Dockerfile .
```

**Running tests:**

```shell
# Start a persistent container in background (8GB memory, 4 CPUs for compilation)
$ container run -d --name rustpython-test -m 8G -c 4 \
    --mount type=bind,source=$(pwd),target=/workspace \
    -w /workspace rustpython-dev sleep infinity

# Run tests inside the container
$ container exec rustpython-test sh -c "cargo run --release -- -m test test_ensurepip"

# Run any command
$ container exec rustpython-test sh -c "cargo test --workspace"

# Stop and remove the container when done
$ container rm -f rustpython-test
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

- `crates/compiler/src`: python compilation to bytecode
  - `crates/compiler-core/src`: python bytecode representation in rust structures
- `crates/derive/src` and `crates/derive-impl/src`: Rust language extensions and macros specific to rustpython
- `Lib`: Carefully selected / copied files from CPython sourcecode. This is
   the python side of the standard library.
  - `test`: CPython test suite
- `crates/vm/src`: python virtual machine
  - `builtins`: Builtin functions and types
  - `stdlib`: Standard library parts implemented in rust.
- `src`: using the other subcrates to bring rustpython to life.
- `crates/wasm`: Binary crate and resources for WebAssembly build
- `extra_tests`: extra integration test snippets as a supplement to `Lib/test`.
  Add new RustPython-only regression tests here; do not place new tests under `Lib/test`.

## Understanding Internals

The RustPython workspace includes the `rustpython` top-level crate. The `Cargo.toml`
file in the root of the repo provide configuration of the crate and the
implementation is found in the `src` directory (specifically, `src/lib.rs`).

The top-level `rustpython` binary depends on several lower-level crates including:

- `ruff_python_parser` and `ruff_python_ast` (external dependencies from the Ruff project)
- `rustpython-compiler` (implementation in `crates/compiler/src`)
- `rustpython-vm` (implementation in `crates/vm/src`)

Together, these crates provide the functions of a programming language and
enable a line of code to go through a series of steps:

- parse the line of source code into tokens
- determine if the tokens are valid syntax
- create an Abstract Syntax Tree (AST)
- compile the AST into bytecode
- execute the bytecode in the virtual machine (VM).

### Parser and AST

RustPython uses the Ruff project's parser and AST implementation:

- Parser: `ruff_python_parser` is used to convert Python source code into tokens
  and parse them into an Abstract Syntax Tree (AST)
- AST: `ruff_python_ast` provides the Rust types and expressions represented by
  the AST nodes
- These are external dependencies maintained by the Ruff project
- For more information, visit the [Ruff GitHub repository](https://github.com/astral-sh/ruff)

### rustpython-compiler

The `rustpython-compiler` crate's purpose is to transform the AST (Abstract Syntax
Tree) to bytecode. The implementation of the compiler is found in the
`crates/compiler/src` directory. The compiler implements Python's symbol table,
ast->bytecode compiler, and bytecode optimizer in Rust.

Implementation of bytecode structure in Rust is found in the `crates/compiler-core/src`
directory. `crates/compiler-core/src/bytecode.rs` contains the representation of
instructions and operations in Rust. Further information about Python's
bytecode instructions can be found in the
[Python documentation](https://docs.python.org/3/library/dis.html#bytecodes).

### rustpython-vm

The `rustpython-vm` crate has the important job of running the virtual machine that
executes Python's instructions. The `crates/vm/src` directory contains code to
implement the read and evaluation loop that fetches and dispatches
instructions. This directory also contains the implementation of the
Python Standard Library modules in Rust (`crates/vm/src/stdlib`). In Python
everything can be represented as an object. The `crates/vm/src/builtins` directory holds
the Rust code used to represent different Python objects and their methods. The
core implementation of what a Python object is can be found in
`crates/vm/src/object/core.rs`.

### Code generation

There are some code generations involved in building RustPython:

- some part of the AST code is generated from `vm/src/stdlib/ast/gen.rs` to `compiler/ast/src/ast_gen.rs`.
- the `__doc__` attributes are generated by the 
  [__doc__](https://github.com/RustPython/__doc__) project which is then included as the `rustpython-doc` crate.

## Questions

Have you tried these steps and have a question, please chat with us on
[Discord](https://discord.gg/vru8NypEhv).
