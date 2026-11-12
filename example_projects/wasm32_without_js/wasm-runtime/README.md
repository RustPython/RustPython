# Native non-JS WASM host

See the [example README](../README.md) for build, CLI, and test commands.

The guest imports `env.print(ptr: i32, len: i32) -> i32`. The host writes that
UTF-8 message to stdout followed by a newline.

The guest exports:

- `memory`
- `alloc_source(len: i32) -> i32`
- `eval(ptr: i32, len: i32) -> i32`

`eval` takes ownership of the `alloc_source` buffer and frees it. It returns 0
on success, 1 on a Python exception, or 2 for invalid UTF-8.
