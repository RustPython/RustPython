//! A crate to hold types and functions common to all rustpython components.

pub mod atomic;
pub mod borrow;
pub mod boxvec;
pub mod bytes;
pub mod char;
pub mod cmp;
pub mod encodings;
pub mod float_ops;
pub mod hash;
pub mod linked_list;
pub mod lock;
pub mod rc;
pub mod refcount;
pub mod static_cell;
pub mod str;

pub mod vendored {
    pub use ascii;
}
