# RustPython wasm32 build without JS

This example runs Python source on `wasm32-unknown-unknown` in a native Wasmer
host. The guest has no JavaScript or WASI dependency and no operating-system
services. It imports `env.print`, `env.kv_get`, and `env.kv_put`. The host
rejects any other import. The `kv` module (`kv.get` / `kv.put`) reads and
writes byte storage owned by the host; the map is empty for each run.

## Build and run

Install the `wasm32-unknown-unknown` Rust target, then build the guest from its
own directory so that Cargo reads its `.cargo/config.toml`:

```shell
rustup target add wasm32-unknown-unknown
(cd rustpython-without-js && cargo build)
cargo build --release --manifest-path wasm-runtime/Cargo.toml
```

```shell
wasm-runtime/target/release/wasm-runtime \
  rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm \
  -c '1 + 3'
wasm-runtime/target/release/wasm-runtime \
  rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm \
  example.py
```

Guest output is written to stdout. Exit status is 0 on success, 1 for a Python
exception, and 2 for invalid input or a host/runtime error.

## Contract tests

```shell
python3 test_runtime.py -v \
  --runtime wasm-runtime/target/release/wasm-runtime \
  --module rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm
```

The existing WASM CI job runs these checks.

This is a no-OS build: `Interpreter::builder` with a native `kv` module, host-lent
storage, a fixed hash seed, and unsupported entropy.
