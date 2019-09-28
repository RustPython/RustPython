#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-parser/")]

#[macro_use]
extern crate log;
use lalrpop_util::lalrpop_mod;

pub mod ast;
pub mod error;
mod fstring;
mod function;
pub mod lexer;
pub mod location;
pub mod parser;
lalrpop_mod!(
    #[allow(clippy::all)]
    python
);
pub mod token;
