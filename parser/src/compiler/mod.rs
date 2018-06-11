// This file makes this directory a submodule.

mod parser;
mod python;
mod ast;
mod token;
mod lexer;
mod compile;
pub mod compile_py_code_object;
mod bytecode;
mod builtins;
mod pyobject;
mod vm;

pub use self::parser::parse;
pub use self::vm::evaluate;


// Mimic eval code objects:
//pub fn eval_code() -> pyobject::PyObjectRef {

//}
