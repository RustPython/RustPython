//! Runtime-independent CPython-compatible Unicode semantics and data.
//!
//! Every entry point operates on plain `char`/`u32`/`CodePoint`/`&Wtf8` values
//! so it can be shared by any Python runtime; argument extraction and Python
//! exception mapping stay with the caller. There is no global mutable state and
//! results depend only on inputs.

#![no_std]

extern crate alloc;

pub mod case;
pub mod classify;
pub mod data;
pub mod identifier;
pub mod normalize;

pub use data::{Ucd, character_name, lookup_character, unicode_version};
pub use normalize::{NormalizeForm, is_normalized, normalize};
