# Vendored wasi-libc emulation library

`libwasi-emulated-signal.a` is a prebuilt static library from
[wasi-sdk](https://github.com/WebAssembly/wasi-sdk), providing wasi-libc's
userspace emulation of `signal()`/`raise()` for `wasm32-wasip1` (WebAssembly
has no asynchronous signal delivery, so this is a synchronous, in-process
handler table, not a real OS signal mechanism).

CPython's own WASI build links the same library
(`configure.ac`'s `_WASI_EMULATED_SIGNAL` case).

- Source: https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sysroot-34.0.tar.gz
  (path: `lib/wasm32-wasip1/libwasi-emulated-signal.a`)
- License: Apache-2.0 WITH LLVM-exception (same as wasi-libc)
- sha256: see below
becc35b856608fb9282d94c70166796e4d7b21e4e1057e625725306e765eb035  crates/host_env/vendor/wasm32-wasip1/libwasi-emulated-signal.a
