# Vendored wasi-libc signal emulation (WASI p2)

`libwasi-emulated-signal.a` is the **wasm32-wasip2** archive from the
[official wasi-sdk 34 sysroot](https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sysroot-34.0.tar.gz),
not a copy of the preview1 archive.

- Archive member: `wasi-sysroot-34.0/lib/wasm32-wasip2/libwasi-emulated-signal.a`
- SHA-256: `49b9844a3a29ea00e4adb58338bbf7ef8f2053078646e78c77ac9a2952deebf6`
- License: Apache-2.0 WITH LLVM-exception, as distributed by
  [wasi-libc](https://github.com/WebAssembly/wasi-libc/blob/main/LICENSE).

This is wasi-libc's synchronous, in-process `signal()`/`raise()` handler
emulation. It does **not** supply asynchronous OS signal delivery. The p2
linker needs it because the shared WASI host signal adapter uses those APIs;
the ordinary libc bundled with the tested Rust toolchain does not provide them.

To reproduce the vendored bytes, extract the member above from the official
release tarball and verify its SHA-256. The preview1 archive from the same
release has a different checksum and remains unchanged.
