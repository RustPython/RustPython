//! Arbitrary-precision decimal arithmetic following the
//! [General Decimal Arithmetic Specification](https://speleotrove.com/decimal/decarith.html).
//!
//! The engine mirrors libmpdec (CPython's `_decimal`) semantics: an operation
//! never raises on its own, it accumulates condition bits into a status word
//! that the caller merges into a context. Algorithms are ported from CPython's
//! `Lib/_pydecimal.py`, which is the executable specification for the parts of
//! the API (correctly rounded transcendentals in particular) that the
//! specification leaves to the implementation.

#![allow(clippy::must_use_candidate)]

extern crate alloc;

pub mod bigops;
mod context;
mod dec;
pub mod fmt;
pub mod ops;
pub mod transcendental;

pub use context::{Context, RoundMode};
pub use dec::{Decimal, ParseError, Special};

/// Condition (status) flags, matching libmpdec's `MPD_*` bit values.
pub mod status {
    pub const CLAMPED: u32 = 1 << 0;
    pub const CONVERSION_SYNTAX: u32 = 1 << 1;
    pub const DIVISION_BY_ZERO: u32 = 1 << 2;
    pub const DIVISION_IMPOSSIBLE: u32 = 1 << 3;
    pub const DIVISION_UNDEFINED: u32 = 1 << 4;
    pub const FPU_ERROR: u32 = 1 << 5;
    pub const INEXACT: u32 = 1 << 6;
    pub const INVALID_CONTEXT: u32 = 1 << 7;
    pub const INVALID_OPERATION: u32 = 1 << 8;
    pub const MALLOC_ERROR: u32 = 1 << 9;
    pub const FLOAT_OPERATION: u32 = 1 << 10;
    pub const OVERFLOW: u32 = 1 << 11;
    pub const ROUNDED: u32 = 1 << 12;
    pub const SUBNORMAL: u32 = 1 << 13;
    pub const UNDERFLOW: u32 = 1 << 14;

    /// Every condition that surfaces as `InvalidOperation`.
    pub const IEEE_INVALID_OPERATION: u32 = CONVERSION_SYNTAX
        | DIVISION_IMPOSSIBLE
        | DIVISION_UNDEFINED
        | FPU_ERROR
        | INVALID_CONTEXT
        | INVALID_OPERATION
        | MALLOC_ERROR;
    pub const ERRORS: u32 = IEEE_INVALID_OPERATION | DIVISION_BY_ZERO;
    pub const TRAPS: u32 = ERRORS | OVERFLOW | UNDERFLOW;
    pub const MAX_STATUS: u32 = (1 << 15) - 1;
}

/// Largest allowed value of `Context.prec`.
pub const MAX_PREC: i64 = 999_999_999_999_999_999;
/// Largest allowed value of `Context.Emax`.
pub const MAX_EMAX: i64 = 999_999_999_999_999_999;
/// Smallest allowed value of `Context.Emin`.
pub const MIN_EMIN: i64 = -999_999_999_999_999_999;
/// Smallest allowed value of `Context.Etiny()`.
pub const MIN_ETINY: i64 = MIN_EMIN - (MAX_PREC - 1);
/// Largest `bits` argument accepted by `IEEEContext`.
pub const IEEE_CONTEXT_MAX_BITS: u32 = 512;
