# RustPython wasm32 build without JS

This example runs Python source on `wasm32-unknown-unknown` in a native Wasmer
host. The guest has no JavaScript or WASI dependency and no operating-system
services. It imports only `env.print`, which sends an evaluation result or Python
exception to the host. The host rejects any other import.

## Build and run

Install the `wasm32-unknown-unknown` Rust target, then build the guest from its
own directory so that Cargo reads its `.cargo/config.toml`:

```shell
rustup target add wasm32-unknown-unknown
(cd rustpython-without-js && cargo build)
cargo build --release --manifest-path wasm-runtime/Cargo.toml
```

Run the default `1 + 3` expression, a command, or a UTF-8 source file:

```shell
wasm-runtime/target/release/wasm-runtime rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm
wasm-runtime/target/release/wasm-runtime rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm -c 'assert 6 * 7 == 42; "passed"'
wasm-runtime/target/release/wasm-runtime rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm example.py --timeout 30
```

The host writes exactly one JSON object to stdout, for example:

```json
{"status":"ok","output":"4"}
```

`output` contains the representation of the last expression (or `None`), the
Python exception, or a host diagnostic. Exit status is 0 for success, 1 for a
Python exception, 2 for invalid input or a runtime error, and 124 for a timeout.
The corresponding JSON statuses are `ok`, `python_error`, `input_error`,
`runtime_error`, and `timeout`. The deadline defaults to 60 seconds and includes
input loading, module compilation, and execution. Each invocation creates a fresh interpreter.
Source is limited to 16 MiB. Files must be UTF-8; encoding cookies are not used.

## Contract tests

Use host CPython to run the tests against the actual guest:

```shell
python3 test_runtime.py -v \
  --runtime wasm-runtime/target/release/wasm-runtime \
  --module rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm
```

The existing WASM CI job runs these checks. They verify supplied source, UTF-8,
32-bit/no-OS identity, Python failures, malformed ABI, traps, unexpected imports,
and host-enforced timeouts. No additional runner or browser is needed.

This is the no-OS execution layer, **not** a `python -m test` implementation or a
CPython regrtest worker. The guest uses `Interpreter::without_stdlib`: it does not
provide normal Python file I/O, stdout/stderr, module discovery, clocks, or
entropy. It uses a fixed hash seed for reproducibility, and its `getrandom`
backend reports unsupported rather than returning uninitialized bytes. A
future regrtest adapter must provide its required capabilities explicitly in a
separate test build, while retaining this no-OS build and import check.
