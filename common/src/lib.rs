//! A crate to hold types and functions common to all rustpython components.

#[macro_use]
mod macros;
pub use macros::*;

pub mod atomic;
pub mod borrow;
pub mod boxvec;
pub mod bytes;
pub mod cformat;
pub mod char;
pub mod cmp;
#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod crt_fd;
pub mod encodings;
pub mod float_ops;
pub mod format;
pub mod hash;
pub mod linked_list;
pub mod lock;
pub mod os;
pub mod rc;
pub mod refcount;
pub mod static_cell;
pub mod str;
pub mod brc;
#[cfg(windows)]
pub mod windows;

pub mod vendored {
    pub use ascii;
}
