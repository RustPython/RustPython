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


## Opt-in host descriptors (no WASI)

The default guest above remains no-OS. A separate opt-in `host-io` guest feature
imports the versioned `rustpython_host_io_v1` ABI. Build it with:

```shell
(cd rustpython-without-js && cargo build --features host-io)
wasm-runtime/target/release/wasm-runtime \
  rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm \
  --host-file existing-file -c "import _io; _io.FileIO(3, 'rb').read()"
```

`--host-file` opens exactly that existing file read/write and lends it as guest
FD 3. It does not create or truncate the file on startup. Guest FD numbers never
refer to arbitrary native descriptors; even 0, 1 and 2 are absent. This is an
explicit read/write capability, not a read-only view. Without the flag the runner
rejects the host descriptor imports. The host limits each transfer to 64 KiB;
Python buffered I/O handles short transfers normally.

The feature runs real `_io.FileIO` over host descriptors, with ownership, EOF,
seek/truncate, and guest errno translation. It does **not** implement path opening,
filesystem discovery, `os`, processes, threads, or `-m test`. Those remain separate
prerequisites; no standard-library tests are monkey-patched or reported as passing.

```shell
cargo test --manifest-path wasm-runtime/Cargo.toml
python3 test_host_io.py -v \
  --runtime wasm-runtime/target/release/wasm-runtime \
  --module rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm
```


## Opt-in read-only path opening

The `host-fs` guest feature additionally imports `rustpython_host_fs_v1.path_open`.
An explicit directory capability supplies relative paths; it never uses the host
working directory implicitly:

```shell
(cd rustpython-without-js && cargo build --features host-fs)
wasm-runtime/target/release/wasm-runtime \
  rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm \
  --host-root directory -c "import _io; _io.FileIO('source.py', 'rb').read()"
```

Paths are resolved with cap-std relative to this preopened directory. Absolute
paths and parent components are rejected. Internal symlinks are allowed, but a
symlink cannot grant access outside the directory. Opened handles are read-only;
create, write and truncate modes return EROFS. `--host-file` may separately lend
an explicit read/write file, so omitting it matters for a read-only embedding.
Unix filename bytes are preserved. Windows paths currently require valid UTF-8;
unpaired UTF-16 surrogate names are explicitly unsupported, not silently altered.

This stage enables path-based FileIO only. Metadata queries and directory
operations are not implemented by this capability provider or guest ABI.
The guest still lacks filesystem `posix` hooks and external importlib integration;
this is not a working `-m test` runner.
