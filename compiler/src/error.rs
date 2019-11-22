use rustpython_parser::error::{LexicalErrorType, ParseError, ParseErrorType};
use rustpython_parser::location::Location;
use rustpython_parser::token::Tok;

use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct CompileError {
    pub statement: Option<String>,
    pub error: CompileErrorType,
    pub location: Location,
}

impl CompileError {
    pub fn update_statement_info(&mut self, statement: String) {
        self.statement = Some(statement);
    }
}

impl From<ParseError> for CompileError {
    fn from(error: ParseError) -> Self {
        CompileError {
            statement: None,
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
                    *token == Tok::Indent || expected.clone() == Some("Indent".to_string())
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
        let error_desc = match &self.error {
            CompileErrorType::Assign(target) => format!("can't assign to {}", target),
            CompileErrorType::Delete(target) => format!("can't delete {}", target),
            CompileErrorType::ExpectExpr => "Expecting expression, got statement".to_string(),
            CompileErrorType::Parse(err) => err.to_string(),
            CompileErrorType::SyntaxError(err) => err.to_string(),
            CompileErrorType::StarArgs => "Two starred expressions in assignment".to_string(),
            CompileErrorType::InvalidBreak => "'break' outside loop".to_string(),
            CompileErrorType::InvalidContinue => "'continue' outside loop".to_string(),
            CompileErrorType::InvalidReturn => "'return' outside function".to_string(),
            CompileErrorType::InvalidYield => "'yield' outside function".to_string(),
        };

        if self.statement.is_some() && self.location.column() > 0 {
            // visualize the error, when location and statement are provided
            write!(
                f,
                "\n{}\n{}",
                self.statement.clone().unwrap(),
                self.location.visualize(&error_desc)
            )
        } else {
            // print line number
            write!(f, "{} at {}", error_desc, self.location)
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
