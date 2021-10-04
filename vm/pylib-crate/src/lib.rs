//! This crate includes the compiled python bytecode of the RustPython standard library. The most
//! common way to use this crate is to just add the `"freeze-stdlib"` feature to `rustpython-vm`,
//! in order to automatically include the python part of the standard library into the binary.

// windows needs to read the symlink out of `Lib` as git turns it into a text file,
// so build.rs sets this env var
pub const LIB_PATH: &str = match option_env!("win_lib_path") {
    Some(s) => s,
    None => concat!(env!("CARGO_MANIFEST_DIR"), "/Lib"),
};

#[cfg(feature = "compiled-bytecode")]
use rustpython_bytecode::FrozenModule;
#[cfg(feature = "compiled-bytecode")]
pub fn frozen_stdlib() -> impl Iterator<Item = (String, FrozenModule)> {
    rustpython_derive::py_freeze!(dir = "Lib", crate_name = "rustpython_bytecode")
}
