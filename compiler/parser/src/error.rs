//! Define internal parse error types
//! The goal is to provide a matching and a safe error API, maksing errors from LALR

use crate::{ast::Location, token::Tok};
use std::fmt;

/// Represents an error during lexical scanning.
#[derive(Debug, PartialEq)]
pub struct LexicalError {
    pub error: LexicalErrorType,
    pub location: Location,
}

impl LexicalError {
    pub fn new(error: LexicalErrorType, location: Location) -> Self {
        Self { error, location }
    }
}

#[derive(Debug, PartialEq)]
pub enum LexicalErrorType {
    StringError,
    UnicodeError,
    NestingError,
    IndentationError,
    TabError,
    TabsAfterSpaces,
    DefaultArgumentError,
    DuplicateArgumentError(String),
    PositionalArgumentError,
    UnpackedArgumentError,
    DuplicateKeywordArgumentError(String),
    UnrecognizedToken { tok: char },
    FStringError(FStringErrorType),
    LineContinuationError,
    Eof,
    OtherError(String),
}

impl fmt::Display for LexicalErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LexicalErrorType::StringError => write!(f, "Got unexpected string"),
            LexicalErrorType::FStringError(error) => write!(f, "f-string: {error}"),
            LexicalErrorType::UnicodeError => write!(f, "Got unexpected unicode"),
            LexicalErrorType::NestingError => write!(f, "Got unexpected nesting"),
            LexicalErrorType::IndentationError => {
                write!(f, "unindent does not match any outer indentation level")
            }
            LexicalErrorType::TabError => {
                write!(f, "inconsistent use of tabs and spaces in indentation")
            }
            LexicalErrorType::TabsAfterSpaces => {
                write!(f, "Tabs not allowed as part of indentation after spaces")
            }
            LexicalErrorType::DefaultArgumentError => {
                write!(f, "non-default argument follows default argument")
            }
            LexicalErrorType::DuplicateArgumentError(arg_name) => {
                write!(f, "duplicate argument '{arg_name}' in function definition")
            }
            LexicalErrorType::DuplicateKeywordArgumentError(arg_name) => {
                write!(f, "keyword argument repeated: {arg_name}")
            }
            LexicalErrorType::PositionalArgumentError => {
                write!(f, "positional argument follows keyword argument")
            }
            LexicalErrorType::UnpackedArgumentError => {
                write!(
                    f,
                    "iterable argument unpacking follows keyword argument unpacking"
                )
            }
            LexicalErrorType::UnrecognizedToken { tok } => {
                write!(f, "Got unexpected token {tok}")
            }
            LexicalErrorType::LineContinuationError => {
                write!(f, "unexpected character after line continuation character")
            }
            LexicalErrorType::Eof => write!(f, "unexpected EOF while parsing"),
            LexicalErrorType::OtherError(msg) => write!(f, "{msg}"),
        }
    }
}

// TODO: consolidate these with ParseError
#[derive(Debug, PartialEq)]
pub struct FStringError {
    pub error: FStringErrorType,
    pub location: Location,
}

impl FStringError {
    pub fn new(error: FStringErrorType, location: Location) -> Self {
        Self { error, location }
    }
}

impl From<FStringError> for LexicalError {
    fn from(err: FStringError) -> Self {
        LexicalError {
            error: LexicalErrorType::FStringError(err.error),
            location: err.location,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum FStringErrorType {
    UnclosedLbrace,
    UnopenedRbrace,
    ExpectedRbrace,
    InvalidExpression(Box<ParseErrorType>),
    InvalidConversionFlag,
    EmptyExpression,
    MismatchedDelimiter(char, char),
    ExpressionNestedTooDeeply,
    ExpressionCannotInclude(char),
    SingleRbrace,
    Unmatched(char),
    UnterminatedString,
}

impl fmt::Display for FStringErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FStringErrorType::UnclosedLbrace => write!(f, "expecting '}}'"),
            FStringErrorType::UnopenedRbrace => write!(f, "Unopened '}}'"),
            FStringErrorType::ExpectedRbrace => write!(f, "Expected '}}' after conversion flag."),
            FStringErrorType::InvalidExpression(error) => {
                write!(f, "{error}")
            }
            FStringErrorType::InvalidConversionFlag => write!(f, "invalid conversion character"),
            FStringErrorType::EmptyExpression => write!(f, "empty expression not allowed"),
            FStringErrorType::MismatchedDelimiter(first, second) => write!(
                f,
                "closing parenthesis '{second}' does not match opening parenthesis '{first}'"
            ),
            FStringErrorType::SingleRbrace => write!(f, "single '}}' is not allowed"),
            FStringErrorType::Unmatched(delim) => write!(f, "unmatched '{delim}'"),
            FStringErrorType::ExpressionNestedTooDeeply => {
                write!(f, "expressions nested too deeply")
            }
            FStringErrorType::UnterminatedString => {
                write!(f, "unterminated string")
            }
            FStringErrorType::ExpressionCannotInclude(c) => {
                if *c == '\\' {
                    write!(f, "f-string expression part cannot include a backslash")
                } else {
                    write!(f, "f-string expression part cannot include '{c}'s")
                }
            }
        }
    }
}

/// Represents an error during parsing
pub type ParseError = rustpython_compiler_core::BaseError<ParseErrorType>;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ParseErrorType {
    /// Parser encountered an unexpected end of input
    Eof,
    /// Parser encountered an extra token
    ExtraToken(Tok),
    /// Parser encountered an invalid token
    InvalidToken,
    /// Parser encountered an unexpected token
    UnrecognizedToken(Tok, Option<String>),
    /// Maps to `User` type from `lalrpop-util`
    Lexical(LexicalErrorType),
}

impl fmt::Display for ParseErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ParseErrorType::Eof => write!(f, "Got unexpected EOF"),
            ParseErrorType::ExtraToken(ref tok) => write!(f, "Got extraneous token: {tok:?}"),
            ParseErrorType::InvalidToken => write!(f, "Got invalid token"),
            ParseErrorType::UnrecognizedToken(ref tok, ref expected) => {
                if *tok == Tok::Indent {
                    write!(f, "unexpected indent")
                } else if expected.as_deref() == Some("Indent") {
                    write!(f, "expected an indented block")
                } else {
                    write!(f, "invalid syntax. Got unexpected token {tok}")
                }
            }
            ParseErrorType::Lexical(ref error) => write!(f, "{error}"),
        }
    }
}

impl ParseErrorType {
    pub fn is_indentation_error(&self) -> bool {
        match self {
            ParseErrorType::Lexical(LexicalErrorType::IndentationError) => true,
            ParseErrorType::UnrecognizedToken(token, expected) => {
                *token == Tok::Indent || expected.clone() == Some("Indent".to_owned())
            }
            _ => false,
        }
    }
    pub fn is_tab_error(&self) -> bool {
        matches!(
            self,
            ParseErrorType::Lexical(LexicalErrorType::TabError)
                | ParseErrorType::Lexical(LexicalErrorType::TabsAfterSpaces)
        )
    }
}
