<img src="./logo.png" width="125" height="125" align="right" />

# [RustPython](https://rustpython.github.io/)

A Python-3 (CPython >= 3.13.0) Interpreter written in Rust :snake: :scream:
:metal:.

[![Build Status](https://github.com/RustPython/RustPython/workflows/CI/badge.svg)](https://github.com/RustPython/RustPython/actions?query=workflow%3ACI)
[![codecov](https://codecov.io/gh/RustPython/RustPython/branch/main/graph/badge.svg)](https://codecov.io/gh/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](https://opensource.org/licenses/MIT)
[![Contributors](https://img.shields.io/github/contributors/RustPython/RustPython.svg)](https://github.com/RustPython/RustPython/graphs/contributors)
[![Discord Shield](https://discordapp.com/api/guilds/1043121930691149845/widget.png?style=shield)][discord]
[![docs.rs](https://docs.rs/rustpython/badge.svg)](https://docs.rs/rustpython/)
[![Crates.io](https://img.shields.io/crates/v/rustpython)](https://crates.io/crates/rustpython)
[![dependency status](https://deps.rs/crate/rustpython/0.1.1/status.svg)](https://deps.rs/crate/rustpython/0.1.1)
[![Open in Gitpod](https://img.shields.io/static/v1?label=Open%20in&message=Gitpod&color=1aa6e4&logo=gitpod)](https://gitpod.io#https://github.com/RustPython/RustPython)

## Usage

**Check out our [online demo](https://rustpython.github.io/demo/) running on WebAssembly.**

RustPython requires Rust latest stable version (e.g 1.67.1 at February 7th 2023). If you don't
currently have Rust installed on your system you can do so by following the instructions at [rustup.rs](https://rustup.rs/).

To check the version of Rust you're currently running, use `rustc --version`. If you wish to update,
`rustup update stable` will update your Rust installation to the most recent stable release.

To build RustPython locally, first, clone the source code:

```bash
git clone https://github.com/RustPython/RustPython
```

RustPython uses symlinks to manage python libraries in `Lib/`. If on windows, running the following helps:
```bash
git config core.symlinks true
```

Then you can change into the RustPython directory and run the demo (Note: `--release` is
needed to prevent stack overflow on Windows):

```bash
$ cd RustPython
$ cargo run --release demo_closures.py
Hello, RustPython!
```

Or use the interactive shell:

```bash
$ cargo run --release
Welcome to rustpython
>>>>> 2+2
4
```

NOTE: For windows users, please set `RUSTPYTHONPATH` environment variable as `Lib` path in project directory.
(e.g. When RustPython directory is `C:\RustPython`, set `RUSTPYTHONPATH` as `C:\RustPython\Lib`)

You can also install and run RustPython with the following:

```bash
$ cargo install --git https://github.com/RustPython/RustPython rustpython
$ rustpython
Welcome to the magnificent Rust Python interpreter
>>>>>
```

### venv

Because RustPython currently doesn't provide a well-packaged installation, using venv helps to use pip easier.

```sh
$ rustpython -m venv <your_env_name>
$ . <your_env_name>/bin/activate
$ python # now `python` is the alias of the RustPython for the new env
```

### PIP

If you'd like to make https requests, you can enable the `ssl` feature, which
also lets you install the `pip` package manager. Note that on Windows, you may
need to install OpenSSL, or you can enable the `ssl-vendor` feature instead,
which compiles OpenSSL for you but requires a C compiler, perl, and `make`.
OpenSSL version 3 is expected and tested in CI. Older versions may not work.

Once you've installed rustpython with SSL support, you can install pip by
running:

```bash
cargo install --git https://github.com/RustPython/RustPython --features ssl
rustpython --install-pip
```

You can also install RustPython through the `conda` package manager, though
this isn't officially supported and may be out of date:

```bash
conda install rustpython -c conda-forge
rustpython
```

### WASI

You can compile RustPython to a standalone WebAssembly WASI module so it can run anywhere.

Build

```bash
cargo build --target wasm32-wasip1 --no-default-features --features freeze-stdlib,stdlib --release
```

Run by wasmer

```bash
wasmer run --dir `pwd` -- target/wasm32-wasip1/release/rustpython.wasm `pwd`/extra_tests/snippets/stdlib_random.py
```

Run by wapm

```bash
$ wapm install rustpython
$ wapm run rustpython
>>>>> 2+2
4
```

#### Building the WASI file

You can build the WebAssembly WASI file with:

```bash
cargo build --release --target wasm32-wasip1 --features="freeze-stdlib"
```

> Note: we use the `freeze-stdlib` to include the standard library inside the binary. You also have to run once `rustup target add wasm32-wasip1`.

### JIT (Just in time) compiler

RustPython has a **very** experimental JIT compiler that compile python functions into native code.

#### Building

By default the JIT compiler isn't enabled, it's enabled with the `jit` cargo feature.

```bash
cargo run --features jit
```

This requires autoconf, automake, libtool, and clang to be installed.

#### Using

To compile a function, call `__jit__()` on it.

```python
def foo():
    a = 5
    return 10 + a

foo.__jit__()  # this will compile foo to native code and subsequent calls will execute that native code
assert foo() == 15
```

## Embedding RustPython into your Rust Applications

Interested in exposing Python scripting in an application written in Rust,
perhaps to allow quickly tweaking logic where Rust's compile times would be inhibitive?
Then `examples/hello_embed.rs` and `examples/mini_repl.rs` may be of some assistance.

## Disclaimer

RustPython is in development, and while the interpreter certainly can be used
in interesting use cases like running Python in WASM and embedding into a Rust
project, do note that RustPython is not totally production-ready.

Contribution is more than welcome! See our contribution section for more
information on this.

## Conference videos

Checkout those talks on conferences:

- [FOSDEM 2019](https://www.youtube.com/watch?v=nJDY9ASuiLc)
- [EuroPython 2018](https://www.youtube.com/watch?v=YMmio0JHy_Y)

## Use cases

Although RustPython is a fairly young project, a few people have used it to
make cool projects:

- [GreptimeDB](https://github.com/GreptimeTeam/greptimedb): an open-source, cloud-native, distributed time-series database. Using RustPython for embedded scripting.
- [pyckitup](https://github.com/pickitup247/pyckitup): a game engine written in
  rust.
- [Robot Rumble](https://github.com/robot-rumble/logic/): an arena-based AI competition platform
- [Ruff](https://github.com/charliermarsh/ruff/): an extremely fast Python linter, written in Rust

## Goals

- Full Python-3 environment entirely in Rust (not CPython bindings)
- A clean implementation without compatibility hacks

## Documentation

Currently along with other areas of the project, documentation is still in an
early phase.

You can read the [online documentation](https://docs.rs/rustpython) for the
latest release, or the [user guide](https://rustpython.github.io/docs/).

You can also generate documentation locally by running:

```shell
cargo doc # Including documentation for all dependencies
cargo doc --no-deps --all # Excluding all dependencies
```

Documentation HTML files can then be found in the `target/doc` directory or you can append `--open` to the previous commands to
have the documentation open automatically on your default browser.

For a high level overview of the components, see the [architecture](architecture/architecture.md) document.

## Contributing

Contributions are more than welcome, and in many cases we are happy to guide
contributors through PRs or on Discord. Please refer to the
[development guide](DEVELOPMENT.md) as well for tips on developments.

With that in mind, please note this project is maintained by volunteers, some of
the best ways to get started are below:

Most tasks are listed in the
[issue tracker](https://github.com/RustPython/RustPython/issues). Check issues
labeled with [good first issue](https://github.com/RustPython/RustPython/issues?q=label%3A%22good+first+issue%22+is%3Aissue+is%3Aopen+) if you wish to start coding.

To enhance CPython compatibility, try to increase unittest coverage by checking this article: [How to contribute to RustPython by CPython unittest](https://rustpython.github.io/guideline/2020/04/04/how-to-contribute-by-cpython-unittest.html)

Another approach is to checkout the source code: builtin functions and object
methods are often the simplest and easiest way to contribute.

You can also simply run `python -I whats_left.py` to assist in finding any unimplemented
method.

## Compiling to WebAssembly

[See this doc](wasm/README.md)

## Community

[![Discord Banner](https://discordapp.com/api/guilds/1043121930691149845/widget.png?style=banner2)][discord]

Chat with us on [Discord][discord].

## Code of conduct

Our code of conduct [can be found here](code-of-conduct.md).

## Credit

The initial work was based on
[windelbouwman/rspython](https://github.com/windelbouwman/rspython) and
[shinglyu/RustPython](https://github.com/shinglyu/RustPython)

[discord]: https://discord.gg/vru8NypEhv

## Links

These are some useful links to related projects:

- https://github.com/ProgVal/pythonvm-rust
- https://github.com/shinglyu/RustPython
- https://github.com/windelbouwman/rspython

## License

This project is licensed under the MIT license. Please see the
[LICENSE](LICENSE) file for more details.

The [project logo](logo.png) is licensed under the CC-BY-4.0
license. Please see the [LICENSE-logo](LICENSE-logo) file
for more details.
