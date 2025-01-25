//! A crate to hold types and functions common to all rustpython components.

#![cfg_attr(target_os = "redox", feature(byte_slice_trim_ascii, new_uninit))]

#[macro_use]
mod macros;
pub use macros::*;

pub mod atomic;
pub mod borrow;
pub mod boxvec;
pub mod cmp;
#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod crt_fd;
pub mod encodings;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub mod fileutils;
pub mod float_ops;
pub mod hash;
pub mod int;
pub mod linked_list;
pub mod lock;
pub mod os;
pub mod rc;
pub mod refcount;
pub mod static_cell;
pub mod str;
#[cfg(windows)]
pub mod windows;

pub mod vendored {
    pub use ascii;
}
