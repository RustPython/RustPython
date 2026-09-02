//! A crate to hold types and functions common to all rustpython components.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(clippy::disallowed_methods)]

extern crate alloc;

pub mod atomic;
#[cfg(feature = "binascii")]
pub mod binascii;
pub mod borrow;
pub mod boxvec;
pub mod cformat;
#[cfg(any(feature = "bz2", feature = "lzma", feature = "zlib"))]
pub mod compression;
pub mod encodings;
pub mod float_ops;
pub mod format;
pub mod hash;
#[cfg(feature = "inet")]
pub mod inet;
pub mod int;
#[cfg(feature = "json")]
pub mod json;
pub mod linked_list;
pub mod lock;
pub mod rand;
pub mod rc;
pub mod refcount;
pub mod static_cell;
pub mod str;
pub mod wtf8_index;

pub use rustpython_wtf8 as wtf8;

pub mod vendored {
    pub use ascii;
}
