// This file makes this directory a submodule.

mod parser;
mod python;
mod ast;
mod token;
mod lexer;
mod compile;
mod bytecode;
mod builtins;
mod pyobject;
mod vm;

pub use self::parser::parse;
pub use self::compile::compile;
pub use self::vm::evaluate;


// Mimic eval code objects:
//pub fn eval_code() -> pyobject::PyObjectRef {

//}