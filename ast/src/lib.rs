// above for the macro
#[macro_use]
pub mod map_ast;

mod ast_gen;
mod constant;
mod impls;
mod location;

pub use ast_gen::*;
pub use location::Location;

pub type Suite<U = ()> = Vec<Stmt<U>>;
