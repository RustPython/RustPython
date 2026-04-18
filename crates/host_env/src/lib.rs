extern crate alloc;

#[macro_use]
mod macros;
pub use macros::*;

pub mod os;

#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod crt_fd;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod fileutils;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod fs;
#[cfg(any(unix, windows))]
pub mod locale;

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
#[cfg(unix)]
pub mod pwd;
#[cfg(unix)]
pub mod resource;
#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
pub mod shm;
#[cfg(any(unix, windows))]
pub mod signal;
pub mod time;

#[cfg(any(unix, windows))]
pub mod faulthandler;
#[cfg(windows)]
pub mod mmap;
#[cfg(windows)]
pub mod msvcrt;
#[cfg(any(unix, windows))]
pub mod multiprocessing;
#[cfg(windows)]
pub mod nt;
#[cfg(windows)]
pub mod overlapped;
#[cfg(windows)]
pub mod testconsole;
#[cfg(windows)]
pub mod winapi;
#[cfg(windows)]
pub mod winreg;
#[cfg(windows)]
pub mod wmi;
