//! A crate to hold types and functions common to all rustpython components.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(clippy::disallowed_methods)]

extern crate alloc;

pub mod atomic;
pub mod borrow;
pub mod boxvec;
pub mod cformat;
pub mod encodings;
pub mod float_ops;
pub mod format;
pub mod hash;
pub mod int;
pub mod linked_list;
pub mod lock;
pub mod rand;
pub mod rc;
pub mod refcount;
pub mod static_cell;
pub mod str;

pub use rustpython_wtf8 as wtf8;

pub mod vendored {
    pub use ascii;
}
