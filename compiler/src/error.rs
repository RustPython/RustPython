use rustpython_parser::error::ParseError;
use rustpython_parser::lexer::Location;

use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct CompileError {
    pub error: CompileErrorType,
    pub location: Location,
}

impl From<ParseError> for CompileError {
    fn from(error: ParseError) -> Self {
        CompileError {
            error: CompileErrorType::Parse(error),
            location: Default::default(), // TODO: extract location from parse error!
        }
    }
}

#[derive(Debug)]
pub enum CompileErrorType {
    /// Invalid assignment, cannot store value in target.
    Assign(&'static str),
    /// Invalid delete
    Delete(&'static str),
    /// Expected an expression got a statement
    ExpectExpr,
    /// Parser error
    Parse(ParseError),
    SyntaxError(String),
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
        match &self.error {
            CompileErrorType::Assign(target) => write!(f, "can't assign to {}", target),
            CompileErrorType::Delete(target) => write!(f, "can't delete {}", target),
            CompileErrorType::ExpectExpr => write!(f, "Expecting expression, got statement"),
            CompileErrorType::Parse(err) => write!(f, "{}", err),
            CompileErrorType::SyntaxError(err) => write!(f, "{}", err),
            CompileErrorType::StarArgs => write!(f, "Two starred expressions in assignment"),
            CompileErrorType::InvalidBreak => write!(f, "'break' outside loop"),
            CompileErrorType::InvalidContinue => write!(f, "'continue' outside loop"),
            CompileErrorType::InvalidReturn => write!(f, "'return' outside function"),
            CompileErrorType::InvalidYield => write!(f, "'yield' outside function"),
        }?;

        // Print line number:
        write!(f, " at line {:?}", self.location.row())
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
