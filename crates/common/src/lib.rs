//! A crate to hold types and functions common to all rustpython components.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[macro_use]
mod macros;
pub use macros::*;

pub mod atomic;
pub mod borrow;
pub mod boxvec;
pub mod cformat;
#[cfg(all(feature = "std", any(unix, windows, target_os = "wasi")))]
pub mod crt_fd;
pub mod encodings;
#[cfg(all(feature = "std", any(not(target_arch = "wasm32"), target_os = "wasi")))]
pub mod fileutils;
pub mod float_ops;
pub mod format;
pub mod hash;
pub mod int;
pub mod linked_list;
pub mod lock;
#[cfg(feature = "std")]
pub mod os;
pub mod rand;
pub mod rc;
pub mod refcount;
pub mod static_cell;
pub mod str;
#[cfg(all(feature = "std", windows))]
pub mod windows;

pub use rustpython_wtf8 as wtf8;

pub mod vendored {
    pub use ascii;
}
