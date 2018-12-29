# RustPython

A Python-3 (CPython >= 3.5.0) Interpreter written in Rust.

[![Build Status](https://travis-ci.org/RustPython/RustPython.svg?branch=master)](https://travis-ci.org/RustPython/RustPython)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](https://opensource.org/licenses/MIT)
[![Contributors](https://img.shields.io/github/contributors/RustPython/RustPython.svg)](https://github.com/RustPython/RustPython/graphs/contributors)
[![Gitter](https://badges.gitter.im/RustPython/Lobby.svg)](https://gitter.im/rustpython/Lobby)

## Usage

### Check out our [online demo](https://rustpython.github.io/demo/) running on WebAssembly.

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
    recieve the Python kwargs as the `this` argument.
-   `stdout?`: `(out: string) => void`: A function to replace the native print
    function, by default `console.log`.

## License

This project is licensed under the MIT license.
