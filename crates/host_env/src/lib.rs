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

#[cfg(any(unix, target_os = "wasi"))]
pub mod fcntl;
#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod select;
#[cfg(unix)]
pub mod syslog;
#[cfg(all(unix, not(target_os = "redox"), not(target_os = "ios")))]
pub mod termios;

#[cfg(unix)]
pub mod posix;
#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
pub mod shm;
#[cfg(unix)]
pub mod signal;
pub mod time;

#[cfg(windows)]
pub mod msvcrt;
#[cfg(windows)]
pub mod nt;
#[cfg(windows)]
pub mod winapi;
