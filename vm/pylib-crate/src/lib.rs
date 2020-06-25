//! This crate includes the compiled python bytecode of the RustPython standard library. The most
//! common way to use this crate is to just add the `"freeze-stdlib"` feature to `rustpython-vm`,
//! in order to automatically include the python part of the standard library into the binary.

extern crate self as rustpython_pylib;

use rustpython_bytecode::bytecode::{self, FrozenModule};
use std::collections::HashMap;

use rustpython_derive::py_compile_bytecode as _py_compile_bytecode;
#[macro_export]
macro_rules! py_compile_bytecode {
    ($($arg:tt)*) => {{
        #[macro_use]
        mod __m {
            $crate::_py_compile_bytecode!($($arg)*);
        }
        __proc_macro_call!()
    }};
}

mod __exports {
    pub use maplit::hashmap;
}

pub fn frozen_stdlib() -> HashMap<String, FrozenModule> {
    py_compile_bytecode!(dir = "Lib", crate_name = "rustpython_pylib")
}
