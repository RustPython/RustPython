<img src="./logo.png" width="125" height="125" align="right" />

# RustPython

A Python-3 (CPython >= 3.5.0) Interpreter written in Rust :snake: :scream:
:metal:.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)
[![Build Status](https://dev.azure.com/ryan0463/ryan/_apis/build/status/RustPython.RustPython?branchName=master)](https://dev.azure.com/ryan0463/ryan/_build/latest?definitionId=1&branchName=master)
[![codecov](https://codecov.io/gh/RustPython/RustPython/branch/master/graph/badge.svg)](https://codecov.io/gh/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](https://opensource.org/licenses/MIT)
[![Contributors](https://img.shields.io/github/contributors/RustPython/RustPython.svg)](https://github.com/RustPython/RustPython/graphs/contributors)
[![Gitter](https://badges.gitter.im/RustPython/Lobby.svg)](https://gitter.im/rustpython/Lobby)
[![docs.rs](https://docs.rs/rustpython/badge.svg)](https://docs.rs/rustpython/)
[![Crates.io](https://img.shields.io/crates/v/rustpython)](https://crates.io/crates/rustpython)
[![dependency status](https://deps.rs/crate/rustpython/0.1.1/status.svg)](https://deps.rs/crate/rustpython/0.1.1)

## Usage

#### Check out our [online demo](https://rustpython.github.io/demo/) running on WebAssembly.

RustPython requires Rust latest stable version (e.g 1.38.0 at Oct 1st 2019). 
To check Rust version: `rustc --version` If you wish to update,
`rustup update stable`.

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

You can also install and run RustPython with the following:

    $ cargo install rustpython
    $ rustpython
    Welcome to the magnificent Rust Python interpreter
    >>>>> 


### WASI

You can compile RustPython to a standalone WebAssembly WASI module so it can run anywhere.

```shell
$ wapm install rustpython
$ wapm run rustpython
>>>>> 2+2
4
```

#### Building the WASI file

You can build the WebAssembly WASI file with:

```
cargo build --release --target wasm32-wasi --features="freeze-stdlib"
```

> Note: we use the `freeze-stdlib` to include the standard library inside the binary.

## Disclaimer

RustPython is in a development phase and should not be used in production or a
fault intolerant setting.

Our current build supports only a subset of Python syntax.

Contribution is also more than welcome! See our contribution section for more
information on this.

## Conference videos

Checkout those talks on conferences:

- [FOSDEM 2019](https://www.youtube.com/watch?v=nJDY9ASuiLc)
- [EuroPython 2018](https://www.youtube.com/watch?v=YMmio0JHy_Y)

## Use cases

Allthough rustpython is a very young project, it is already used in the wild:

- [pyckitup](https://github.com/pickitup247/pyckitup): a game engine written in
  rust.
- [codingworkshops.org](https://github.com/chicode/codingworkshops): a site
  where you can learn how to code.

## Goals

- Full Python-3 environment entirely in Rust (not CPython bindings)
- A clean implementation without compatibility hacks

## Documentation

Currently along with other areas of the project, documentation is still in an
early phase.

You can read the [online documentation](https://docs.rs/rustpython-vm) for the
latest release.

You can also generate documentation locally by running:

```shell
$ cargo doc # Including documentation for all dependencies
$ cargo doc --no-deps --all # Excluding all dependencies
```

Documentation HTML files can then be found in the `target/doc` directory.

## Contributing

Contributions are more than welcome, and in many cases we are happy to guide
contributors through PRs or on gitter. Please refer to the
[development guide](DEVELOPMENT.md) as well for tips on developments.

With that in mind, please note this project is maintained by volunteers, some of
the best ways to get started are below:

Most tasks are listed in the
[issue tracker](https://github.com/RustPython/RustPython/issues). Check issues
labeled with `good first issue` if you wish to start coding.

Another approach is to checkout the source code: builtin functions and object
methods are often the simplest and easiest way to contribute.

You can also simply run `./whats_left.sh` to assist in finding any unimplemented
method.

## Using a standard library

As of now the standard library is under construction. You can use a standard
library by setting the RUSTPYTHONPATH environment variable.

To do this, follow this method:

```shell
$ export RUSTPYTHONPATH=~/GIT/RustPython/Lib
$ cargo run -- -c 'import xdrlib'
```

You can play around with other standard libraries for python. For example, the
[ouroboros library](https://github.com/pybee/ouroboros).

## Compiling to WebAssembly

[See this doc](wasm/README.md)

## Community

Chat with us on [gitter][gitter].

## Code of conduct

Our code of conduct [can be found here](code-of-conduct.md).

## Credit

The initial work was based on
[windelbouwman/rspython](https://github.com/windelbouwman/rspython) and
[shinglyu/RustPython](https://github.com/shinglyu/RustPython)

[gitter]: https://gitter.im/rustpython/Lobby

## Links

These are some useful links to related projects:

- https://github.com/ProgVal/pythonvm-rust
- https://github.com/shinglyu/RustPython
- https://github.com/windelbouwman/rspython

## License

This project is licensed under the MIT license. Please see the
[LICENSE](LICENSE) file for more details.
