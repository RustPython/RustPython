//! This crate includes the compiled python bytecode of the RustPython standard library. The most
//! common way to use this crate is to just add the `"freeze-stdlib"` feature to `rustpython-vm`,
//! in order to automatically include the python part of the standard library into the binary.

extern crate self as rustpython_pylib;

pub const LIB_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/Lib");

#[cfg(feature = "compiled-bytecode")]
use {
    rustpython_bytecode::bytecode::{self, FrozenModule},
    std::collections::HashMap,
};
#[cfg(feature = "compiled-bytecode")]
pub fn frozen_stdlib() -> HashMap<String, FrozenModule> {
    rustpython_derive::py_compile_bytecode!(dir = "Lib", crate_name = "rustpython_pylib")
}
