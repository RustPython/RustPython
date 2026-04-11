extern crate alloc;

#[macro_use]
mod macros;
pub use macros::*;

pub mod os;

#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod crt_fd;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod fileutils;

#[cfg(windows)]
pub mod windows;
