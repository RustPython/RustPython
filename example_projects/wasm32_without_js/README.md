# RustPython wasm32 build without JS

To test, build rustpython to wasm32-unknown-unknown target first.

```shell
cd rustpython-without-js  # due to `.cargo/config.toml`
cargo build
cd ..
```

Then there will be `rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm` file.

Now we can run the wasm file with wasm runtime:

```shell
cargo run --release --manifest-path wasm-runtime/Cargo.toml rustpython-without-js/target/wasm32-unknown-unknown/debug/rustpython_without_js.wasm
```

