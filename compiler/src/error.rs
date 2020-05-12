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
    pub source_path: Option<String>,
}

impl CompileError {
    pub fn update_statement_info(&mut self, statement: String) {
        self.statement = Some(statement);
    }

    pub fn update_source_path(&mut self, source_path: &str) {
        debug_assert!(self.source_path.is_none());
        self.source_path = Some(source_path.to_owned());
    }
}

impl From<ParseError> for CompileError {
    fn from(error: ParseError) -> Self {
        CompileError {
            statement: None,
            error: CompileErrorType::Parse(error.error),
            location: error.location,
            source_path: None,
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
    MultipleStarArgs,
    /// Misplaced `*` expression
    InvalidStarExpr,
    /// Break statement outside of loop.
    InvalidBreak,
    /// Continue statement outside of loop.
    InvalidContinue,
    InvalidReturn,
    InvalidYield,
    InvalidYieldFrom,
    InvalidAwait,
    AsyncYieldFrom,
    AsyncReturnValue,
    InvalidFuturePlacement,
    InvalidFutureFeature(String),
}

impl CompileError {
    pub fn is_indentation_error(&self) -> bool {
        if let CompileErrorType::Parse(parse) = &self.error {
            match parse {
                ParseErrorType::Lexical(LexicalErrorType::IndentationError) => true,
                ParseErrorType::UnrecognizedToken(token, expected) => {
                    *token == Tok::Indent || expected.clone() == Some("Indent".to_owned())
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
            CompileErrorType::ExpectExpr => "Expecting expression, got statement".to_owned(),
            CompileErrorType::Parse(err) => err.to_string(),
            CompileErrorType::SyntaxError(err) => err.to_string(),
            CompileErrorType::MultipleStarArgs => {
                "two starred expressions in assignment".to_owned()
            }
            CompileErrorType::InvalidStarExpr => "can't use starred expression here".to_owned(),
            CompileErrorType::InvalidBreak => "'break' outside loop".to_owned(),
            CompileErrorType::InvalidContinue => "'continue' outside loop".to_owned(),
            CompileErrorType::InvalidReturn => "'return' outside function".to_owned(),
            CompileErrorType::InvalidYield => "'yield' outside function".to_owned(),
            CompileErrorType::InvalidYieldFrom => "'yield from' outside function".to_owned(),
            CompileErrorType::InvalidAwait => "'await' outside async function".to_owned(),
            CompileErrorType::AsyncYieldFrom => "'yield from' inside async function".to_owned(),
            CompileErrorType::AsyncReturnValue => {
                "'return' with value inside async generator".to_owned()
            }
            CompileErrorType::InvalidFuturePlacement => {
                "from __future__ imports must occur at the beginning of the file".to_owned()
            }
            CompileErrorType::InvalidFutureFeature(feat) => {
                format!("future feature {} is not defined", feat)
            }
        };

        if let Some(statement) = &self.statement {
            if self.location.column() > 0 {
                if let Some(line) = statement.lines().nth(self.location.row() - 1) {
                    // visualize the error, when location and statement are provided
                    return write!(f, "{}", self.location.visualize(line, &error_desc));
                }
            }
        }

        // print line number
        write!(f, "{} at {}", error_desc, self.location)
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
