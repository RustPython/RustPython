# RustPython Development Guide and Tips

## Code organization

- `bytecode/src`: python bytecode representation in rust structures
- `compiler/src`: python compilation to bytecode
- `parser/src`: python lexing, parsing and ast
- `Lib`: Carefully selected / copied files from CPython sourcecode. This is
   the python side of the standard library.
- `vm/src`: python virtual machine
  - `builtins.rs`: Builtin functions
  - `compile.rs`: the python compiler from ast to bytecode
  - `obj`: python builtin types
  - `stdlib`: Standard library parts implemented in rust.
- `src`: using the other subcrates to bring rustpython to life.
- `docs`: documentation (work in progress)
- `py_code_object`: CPython bytecode to rustpython bytecode converter (work in
  progress)
- `wasm`: Binary crate and resources for WebAssembly build
- `tests`: integration test snippets

## Code style

The code style used is the default
[rustfmt](https://github.com/rust-lang/rustfmt) codestyle. Please format your
code accordingly. We also use [clippy](https://github.com/rust-lang/rust-clippy)
to detect rust code issues.

## Testing

To test rustpython, there is a collection of python snippets located in the
`tests/snippets` directory. To run those tests do the following:

```shell
$ cd tests
$ pytest -v
```

There also are some unit tests, you can run those with cargo:

```shell
$ cargo test --all
```

## Profiling

To profile rustpython, simply build in release mode with the `flame-it` feature.
This will generate a file `flamescope.json`, which you can then view at
https://speedscope.app.

```sh
$ cargo run --release --features flame-it script.py
$ cat flamescope.json
{<json>}
```

You can also pass the `--output-file` option to choose which file to output to
(or stdout if you specify `-`), and the `--output-format` option to choose if
you want to output in the speedscope json format (default), text, or a raw html
viewer (currently broken).
