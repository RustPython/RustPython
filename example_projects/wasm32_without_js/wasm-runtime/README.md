# Native non-JS WASM host

See the [example README](../README.md) for build, CLI, and test commands.

The guest imports:

- `env.print(ptr: i32, len: i32) -> i32`: write a UTF-8 message to stdout
  followed by a newline
- `env.kv_put(key_ptr: i32, key_len: i32, value_ptr: i32, value_len: i32) -> i32`:
  copy the value under the key and return 0
- `env.kv_get(key_ptr: i32, key_len: i32, buf_ptr: i32, buf_len: i32) -> i32`:
  return -1 if the key is missing, otherwise the full value length. The value
  is copied into the buffer only when it fits; otherwise the caller retries
  with a buffer of that length.

The guest exports:

- `memory`
- `alloc_source(len: i32) -> i32`
- `eval(ptr: i32, len: i32) -> i32`

`eval` takes ownership of the `alloc_source` buffer and frees it. It returns 0
on success, 1 on a Python exception, or 2 for invalid UTF-8.
