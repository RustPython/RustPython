# Simple WASM Runtime

WebAssembly runtime POC with wasmer with HashMap-based KV store.
First make sure to install wat2wasm and rust.

```bash
# following command installs rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

cargo run --release <wasm binary>
```

## WASM binary requirements

Entry point is `eval(code_ptr: i32, code_len: i32) -> i32`, following are exported functions, on error return -1:

- `kv_put(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32`
- `kv_get(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32`
- `print(msg_ptr: i32, msg_len: i32) -> i32`
