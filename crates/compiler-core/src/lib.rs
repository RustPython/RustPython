#![no_std]
#![recursion_limit = "256"] // Needed for `define_opcodes!` macro
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-compiler-core/")]

extern crate alloc;

pub mod bytecode;
pub mod frozen;
pub mod marshal;
mod mode;
pub mod varint;

pub use mode::Mode;

pub use ruff_source_file::{
    LineIndex, OneIndexed, PositionEncoding, SourceFile, SourceFileBuilder, SourceLocation,
};
