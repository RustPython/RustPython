# RustPython
A Python-3  (CPython >= 3.5.0) Interpreter written in Rust :snake: :scream: :metal:.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](https://opensource.org/licenses/MIT)

# Usage

To test RustPython, do the following:

    $ git clone https://github.com/RustPython/RustPython
    $ cd RustPython
    $ cargo run demo.py
    Hello, RustPython!

Or use the interactive shell:

    $ cargo run
    Welcome to rustpython
    >>>>> 2+2
    4


# Goals

- Full Python-3 environment entirely in Rust (not CPython bindings)
- A clean implementation without compatibility hacks

# Code organization

- `parser`: python lexing, parsing and ast
- `vm`: python virtual machine
- `src`: using the other subcrates to bring rustpython to life.
- `docs`: documentation (work in progress)
- `py_code_object`: CPython bytecode to rustpython bytecode convertor (work in progress)
- `wasm`: Binary crate and resources for WebAssembly build 
- `tests`: integration test snippets

# Contributing

To start contributing, there are a lot of things that need to be done.
Most tasks are listed in the [issue tracker](https://github.com/RustPython/RustPython/issues).
Another approach is to checkout the sourcecode: builtin functions and object methods are often the simplest
and easiest way to contribute. 

You can also simply run
`cargo run tests/snippets/todo.py` to assist in finding any
unimplemented method.

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

# Compiling to WebAssembly

## Setup

Using `rustup` add the compile target `wasm32-unknown-emscripten`. To do so you will need to have [rustup](https://rustup.rs/) installed.

```bash
rustup target add wasm32-unknown-emscripten
```

Next, install `emsdk`:

```bash
curl https://s3.amazonaws.com/mozilla-games/emscripten/releases/emsdk-portable.tar.gz | tar -zxv
cd emsdk-portable/
./emsdk update
./emsdk install sdk-incoming-64bit
./emsdk activate sdk-incoming-64bit
source ./emsdk_env.sh
```

## Build

Move into the `wasm` directory. This contains a custom binary crate optimized for a web assembly build. 

```bash
cd wasm
```

From here run the build. This can take several minutes depending on the machine.
```
cargo build --target=wasm32-unknown-emscripten --release
```

Upon successful build, the following files will be available:


```
target/wasm32-unknown-emscripten/release/rustpython_wasm.wasm
target/wasm32-unknown-emscripten/release/rustpython_wasm.js
```

- `rustpython_wasm.wasm`: the wasm build for rustpython. It includes both an parser and virtual machine.
- `rustpython_wasm.js`: the loading scripts for the above wasm file.

You will also find `index.html` in the `wasm` directory. 
From here, you can copy these 3 files into the static assets directory of your web browser and you should be
able to see the ouput:

```
Hello RustPython!
```

in the web console.

# Code style

The code style used is the default rustfmt codestyle. Please format your code accordingly.

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

