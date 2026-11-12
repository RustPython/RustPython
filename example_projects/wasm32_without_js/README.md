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
cargo build --locked --release --manifest-path wasm-runtime/Cargo.toml
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


## Opt-in host file I/O (WASI preview1 subset)

The default guest above remains no-OS. A separate opt-in `host-io` guest feature
imports a WASI preview1 subset (`wasi_snapshot_preview1`) for path opening and
descriptor I/O. The host decides access through preopens:

```shell
(cd rustpython-without-js && cargo build --features host-io)
wasm-runtime/target/release/wasm-runtime \
  rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm \
  --dir directory -c "import _io; _io.FileIO('source.py', 'rb').read()"
```

`--dir HOST[::GUEST]` preopens a read-write directory; `--ro-dir` is the
read-only form. `GUEST` defaults to `/`. Guest FD numbers 0, 1 and 2 are
absent; preopens start at 3 in command-line order. Without a preopen the
runner rejects the WASI imports. The host limits each transfer to 64 KiB;
Python buffered I/O handles short transfers normally.

A read-only preopen does not grant write, create, truncate, or similar rights.
`path_open` that asks for an ungranted right, or uses create/truncate/exclusive
or append flags under a read-only preopen, fails without touching the
filesystem. Absolute paths and escapes (`..` past the root, symlinks that
leave the directory) are denied. Paths must be valid UTF-8, as WASI
preview1 requires.

The capability table and guest-memory validation live in the workspace
`rustpython-wasi-host` crate, which depends only on `cap-std` and `std`.
The runner supplies thin Wasmer memory bindings; the shared core does not
depend on Wasmer or the VM. Native `host_io` and browser integration are
separate work and are not enabled by this extraction.

The same contract tests run against this runner and against `wasm32-wasip1`
RustPython under `wasmer`:

```shell
cargo test --locked --manifest-path ../../Cargo.toml -p rustpython-wasi-host
cargo test --locked --release --manifest-path wasm-runtime/Cargo.toml
python3 test_wasi_fs.py -v \
  --runtime wasm-runtime/target/release/wasm-runtime \
  --module rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm
python3 test_wasi_fs.py -v \
  --wasmer "$(command -v wasmer)" \
  --wasi-module ../../target/wasm32-wasip1/wasm-release/rustpython.wasm
```

This stage enables path-based FileIO only. Metadata queries and directory
operations are not implemented. The guest still lacks filesystem `posix` hooks
and external importlib integration; this is not a working `-m test` runner.

### Errno compatibility

The no-JS and browser `wasm32-unknown-unknown` builds use WASI preview1
errno numbers for their supported errno names, matching `wasm32-wasip1`,
rather than the previous BSD numbers. This does not add the full WASI errno
name table; host errors without an exported name still retain their raw value.
Use symbolic `errno.E*` constants instead of numeric literals. `ESHUTDOWN`
is no longer exported on wasm32-unknown: WASI preview1 does not define it.
Native host builds retain their platform errno values.

To compare the errno tables directly, supply both pairs of runtime/module
arguments to `test_wasi_fs.py`. The ordinary tests use the no-JS runner in
that invocation; the comparison executes both guests. Run the wasmer-only
command above separately for the complete WASI contract suite.
