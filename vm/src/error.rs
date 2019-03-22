use rustpython_parser::error::ParseError;

use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum CompileError {
    /// Invalid assignment, cannot store value in target.
    Assign(&'static str),
    /// Invalid delete
    Delete(&'static str),
    /// Expected an expression got a statement
    ExpectExpr,
    /// Parser error
    Parse(ParseError),
    /// Multiple `*` detected
    StarArgs,
    /// Break statement outside of loop.
    InvalidBreak,
    /// Continue statement outside of loop.
    InvalidContinue,
    InvalidReturn,
    InvalidYield,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CompileError::Assign(target) => write!(f, "can't assign to {}", target),
            CompileError::Delete(target) => write!(f, "can't delete {}", target),
            CompileError::ExpectExpr => write!(f, "Expecting expression, got statement"),
            CompileError::Parse(err) => write!(f, "{}", err),
            CompileError::StarArgs => write!(f, "Two starred expressions in assignment"),
            CompileError::InvalidBreak => write!(f, "'break' outside loop"),
            CompileError::InvalidContinue => write!(f, "'continue' outside loop"),
            CompileError::InvalidReturn => write!(f, "'return' outside function"),
            CompileError::InvalidYield => write!(f, "'yield' outside function"),
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
