#[macro_use]
extern crate log;
use lalrpop_util::lalrpop_mod;

pub mod ast;
pub mod error;
mod fstring;
pub mod lexer;
pub mod parser;
lalrpop_mod!(
    #[allow(clippy::all)]
    python
);
pub mod token;
