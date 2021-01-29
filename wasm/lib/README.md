# RustPython

A Python-3 (CPython >= 3.8.0) Interpreter written in Rust.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](https://opensource.org/licenses/MIT)
[![Contributors](https://img.shields.io/github/contributors/RustPython/RustPython.svg)](https://github.com/RustPython/RustPython/graphs/contributors)
[![Gitter](https://badges.gitter.im/RustPython/Lobby.svg)](https://gitter.im/rustpython/Lobby)

# WARNING: this project is still in a pre-alpha state!

**Using this in a production project is inadvisable. Please only do so if you understand the risks.**

## Usage

#### Check out our [online demo](https://rustpython.github.io/demo/) running on WebAssembly.

## Goals

-   Full Python-3 environment entirely in Rust (not CPython bindings)
-   A clean implementation without compatibility hacks

## Quick Documentation

```js
pyEval(code, options?);
```

`code`: `string`: The Python code to run

`options`:

-   `vars?`: `{ [key: string]: any }`: Variables passed to the VM that can be
    accessed in Python with the variable `js_vars`. Functions do work, and
    receive the Python kwargs as the `this` argument.
-   `stdout?`: `"console" | ((out: string) => void) | null`: A function to replace the
    native print function, and it will be `console.log` when giving `undefined`
    or "console", and it will be a dumb function when giving null.

## License

This project is licensed under the MIT license.
