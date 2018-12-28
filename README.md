# RustPython

A Python-3 (CPython >= 3.5.0) Interpreter written in Rust :snake: :scream: :metal:.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](https://opensource.org/licenses/MIT)
[![Contributors](https://img.shields.io/github/contributors/RustPython/RustPython.svg)](https://github.com/RustPython/RustPython/graphs/contributors)
[![Gitter](https://badges.gitter.im/RustPython/Lobby.svg)](https://gitter.im/rustpython/Lobby)

# Usage

### Check out our [online demo](https://rustpython.github.io/demo/) running on WebAssembly.

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

# Documentation

Currently the project is in an early phase, and so is the documentation.

You can read the [online documentation](https://rustpython.github.io/website/rustpython/index.html) for the latest code on master.

You can also generate documentation locally by running:

```shell
$ cargo doc # Including documentation for all dependencies
$ cargo doc --no-deps --all # Excluding all dependencies
```

Documentation HTML files can then be found in the `target/doc` directory.

If you wish to update the online documentation. Push directly to the `release` branch (or ask a maintainer to do so), this will trigger a Travis build that updates the documentation and WebAssembly demo page.

# Code organization

- `parser/src`: python lexing, parsing and ast
- `vm/src`: python virtual machine
  - `builtins.rs`: Builtin functions
  - `compile.rs`: the python compiler from ast to bytecode
  - `obj`: python builtin types
- `src`: using the other subcrates to bring rustpython to life.
- `docs`: documentation (work in progress)
- `py_code_object`: CPython bytecode to rustpython bytecode convertor (work in progress)
- `wasm`: Binary crate and resources for WebAssembly build
- `tests`: integration test snippets

# Contributing

To start contributing, there are a lot of things that need to be done.

Most tasks are listed in the [issue tracker](https://github.com/RustPython/RustPython/issues).
Check issues labeled with `good first issue` if you wish to start coding.

Another approach is to checkout the sourcecode: builtin functions and object methods are often the simplest
and easiest way to contribute.

You can also simply run
`cargo run tests/snippets/whats_left_to_implement.py` to assist in finding any
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

# Using another standard library

As of now the standard library is under construction.

You can play around
with other standard libraries for python. For example,
the [ouroboros library](https://github.com/pybee/ouroboros).

To do this, follow this method:

```shell
$ cd ~/GIT
$ git clone git@github.com:pybee/ouroboros.git
$ export PYTHONPATH=~/GIT/ouroboros/ouroboros
$ cd RustPython
$ cargo run -- -c 'import statistics'
```

# Compiling to WebAssembly

At this stage RustPython only has preliminary support for web assembly. The instructions here are intended for developers or those wishing to run a toy example.

## Setup

To get started, install [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) and `npm`. ([wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/whirlwind-tour/basic-usage.html) should be installed by `wasm-pack`. if not, install it yourself)

<!-- Using `rustup` add the compile target `wasm32-unknown-emscripten`. To do so you will need to have [rustup](https://rustup.rs/) installed.

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
``` -->

## Build

Move into the `wasm` directory. This directory contains a library crate for interop
with python to rust to js and back in `wasm/lib`, the demo website found at
https://rustpython.github.io/demo in `wasm/demo`, and an example of how to use
the crate as a library in one's own JS app in `wasm/example`.

```sh
cd wasm
```

Go to the demo directory. This is the best way of seeing the changes made to either
the library or the JS demo, as the `rustpython_wasm` module is set to the global
JS variable `rp` on the website.

```sh
cd demo
```

Now, start the webpack development server. It'll compile the crate and then
the demo app. This will likely take a long time, both the wasm-pack portion and
the webpack portion (from after it says "Your crate has been correctly compiled"),
so be patient.

```sh
npm run dev
```

You can now open the webpage on https://localhost:8080 and Python code in either
the text box or browser devtools with:

```js
rp.pyEval(
  `
print(js_vars['a'] * 9)
`,
  {
    vars: {
      a: 9
    }
  }
);
```

Alternatively, you can run `npm run build` to build the app once, without watching
for changes, or `npm run dist` to build the app in release mode, both for the
crate and webpack.

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
