//! Errno numbers for `wasm32-unknown-unknown`.
//!
//! There is no libc errno table. The values are the Linux ABI the stdlib
//! and `OSError` subclasses match against.

pub mod errors {
    pub const EAGAIN: i32 = 11;
    pub const ECONNABORTED: i32 = 103;
    pub const ECONNRESET: i32 = 104;
}
