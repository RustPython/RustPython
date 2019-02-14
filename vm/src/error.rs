use rustpython_parser::error::ParseError;

use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum CompileError {
    // Invalid assignment, cannot store value in target.
    Assign(String),
    // Invalid delete
    Delete,
    // Expected an expression got a statement
    ExpectExpr,
    // Parser error
    Parse(ParseError),
    // Multiple `*` detected
    StarArgs,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CompileError::Assign(expr) => write!(f, "Invalid assignment: {}", expr),
            CompileError::Delete => write!(f, "Invalid delete statement"),
            CompileError::ExpectExpr => write!(f, "Expecting expression, got statement"),
            CompileError::Parse(err) => err.fmt(f),
            CompileError::StarArgs => write!(f, "Two starred expressions in assignment"),
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
