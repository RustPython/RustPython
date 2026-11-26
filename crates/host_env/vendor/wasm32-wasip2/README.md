# Vendored wasi-libc emulation library

`libwasi-emulated-signal.a` is a prebuilt static library from
[wasi-sdk](https://github.com/WebAssembly/wasi-sdk), providing wasi-libc's
userspace emulation of `signal()`/`raise()` for `wasm32-wasip2`. It is the
`wasm32-wasip2` build of the library in `../wasm32-wasip1`, from the same
sysroot; the two archives are not byte-identical. `crates/host_env/build.rs`
links it for `wasm32-wasip2`.

- Source: https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sysroot-34.0.tar.gz
  (path: `lib/wasm32-wasip2/libwasi-emulated-signal.a`)
- License: Apache-2.0 WITH LLVM-exception (same as wasi-libc)
- sha256: see below
49b9844a3a29ea00e4adb58338bbf7ef8f2053078646e78c77ac9a2952deebf6  crates/host_env/vendor/wasm32-wasip2/libwasi-emulated-signal.a
