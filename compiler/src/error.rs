use rustpython_parser::error::{LexicalErrorType, ParseError, ParseErrorType};
use rustpython_parser::location::Location;
use rustpython_parser::token::Tok;

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
            error: CompileErrorType::Parse(error.error),
            location: error.location,
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
    Parse(ParseErrorType),
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

impl CompileError {
    pub fn is_indentation_error(&self) -> bool {
        if let CompileErrorType::Parse(parse) = &self.error {
            match parse {
                ParseErrorType::Lexical(LexicalErrorType::IndentationError) => true,
                ParseErrorType::UnrecognizedToken(token, expected) => {
                    if *token == Tok::Indent {
                        true
                    } else if expected.clone() == Some("Indent".to_string()) {
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    pub fn is_tab_error(&self) -> bool {
        if let CompileErrorType::Parse(parse) = &self.error {
            if let ParseErrorType::Lexical(lex) = parse {
                if let LexicalErrorType::TabError = lex {
                    return true;
                }
            }
        }
        false
    }
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
        write!(f, " at {}", self.location)
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
