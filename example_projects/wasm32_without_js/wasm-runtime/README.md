# Native non-JS WASM host

See the [example README](../README.md) for build, CLI, status, and test commands.

The host supplies only `env.print(message_ptr: i32, message_len: i32) -> i32`.
It reads a UTF-8 message from guest memory, validates its bounds, and saves it
for the final JSON record. A malformed pointer or message traps the invocation.
Missing or duplicate result messages are protocol errors. No WASI or JavaScript
imports are provided.

The guest must export:

- `memory`
- `alloc_source(len: i32) -> i32`: allocate a source buffer and return its pointer
- `eval(ptr: i32, len: i32) -> i32`: execute the source, reporting through `env.print`
- `free_source(ptr: i32, len: i32)`: release that allocation after evaluation

`eval` returns 0 on success, 1 on a Python exception, or 2 for invalid UTF-8.
Other values are protocol errors, not successes. Host allocation, execution,
and deallocation traps result in a nonzero process exit. The host deadline also
terminates guests that never return, independently of their Python capabilities.
