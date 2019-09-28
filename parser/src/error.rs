//! Define internal parse error types
//! The goal is to provide a matching and a safe error API, maksing errors from LALR
use lalrpop_util::ParseError as LalrpopError;

use crate::location::Location;
use crate::token::Tok;

use std::error::Error;
use std::fmt;

/// Represents an error during lexical scanning.
#[derive(Debug, PartialEq)]
pub struct LexicalError {
    pub error: LexicalErrorType,
    pub location: Location,
}

#[derive(Debug, PartialEq)]
pub enum LexicalErrorType {
    StringError,
    UnicodeError,
    NestingError,
    PositionalArgumentError,
    DuplicateKeywordArgumentError,
    UnrecognizedToken { tok: char },
    FStringError(FStringErrorType),
    OtherError(String),
}

impl fmt::Display for LexicalErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LexicalErrorType::StringError => write!(f, "Got unexpected string"),
            LexicalErrorType::FStringError(error) => write!(f, "Got error in f-string: {}", error),
            LexicalErrorType::UnicodeError => write!(f, "Got unexpected unicode"),
            LexicalErrorType::NestingError => write!(f, "Got unexpected nesting"),
            LexicalErrorType::DuplicateKeywordArgumentError => {
                write!(f, "keyword argument repeated")
            }
            LexicalErrorType::PositionalArgumentError => {
                write!(f, "positional argument follows keyword argument")
            }
            LexicalErrorType::UnrecognizedToken { tok } => {
                write!(f, "Got unexpected token {}", tok)
            }
            LexicalErrorType::OtherError(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<LexicalError> for LalrpopError<Location, Tok, LexicalError> {
    fn from(err: LexicalError) -> Self {
        lalrpop_util::ParseError::User { error: err }
    }
}

// TODO: consolidate these with ParseError
#[derive(Debug, PartialEq)]
pub struct FStringError {
    pub error: FStringErrorType,
    pub location: Location,
}

#[derive(Debug, PartialEq)]
pub enum FStringErrorType {
    UnclosedLbrace,
    UnopenedRbrace,
    InvalidExpression(Box<ParseErrorType>),
    InvalidConversionFlag,
    EmptyExpression,
    MismatchedDelimiter,
}

impl fmt::Display for FStringErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FStringErrorType::UnclosedLbrace => write!(f, "Unclosed '('"),
            FStringErrorType::UnopenedRbrace => write!(f, "Unopened ')'"),
            FStringErrorType::InvalidExpression(error) => {
                write!(f, "Invalid expression: {}", error)
            }
            FStringErrorType::InvalidConversionFlag => write!(f, "Invalid conversion flag"),
            FStringErrorType::EmptyExpression => write!(f, "Empty expression"),
            FStringErrorType::MismatchedDelimiter => write!(f, "Mismatched delimiter"),
        }
    }
}

impl From<FStringError> for LalrpopError<Location, Tok, LexicalError> {
    fn from(err: FStringError) -> Self {
        lalrpop_util::ParseError::User {
            error: LexicalError {
                error: LexicalErrorType::FStringError(err.error),
                location: err.location,
            },
        }
    }
}

/// Represents an error during parsing
#[derive(Debug, PartialEq)]
pub struct ParseError {
    pub error: ParseErrorType,
    pub location: Location,
}

#[derive(Debug, PartialEq)]
pub enum ParseErrorType {
    /// Parser encountered an unexpected end of input
    EOF,
    /// Parser encountered an extra token
    ExtraToken(Tok),
    /// Parser encountered an invalid token
    InvalidToken,
    /// Parser encountered an unexpected token
    UnrecognizedToken(Tok, Vec<String>),
    /// Maps to `User` type from `lalrpop-util`
    Lexical(LexicalErrorType),
}

/// Convert `lalrpop_util::ParseError` to our internal type
impl From<LalrpopError<Location, Tok, LexicalError>> for ParseError {
    fn from(err: LalrpopError<Location, Tok, LexicalError>) -> Self {
        match err {
            // TODO: Are there cases where this isn't an EOF?
            LalrpopError::InvalidToken { location } => ParseError {
                error: ParseErrorType::EOF,
                location,
            },
            LalrpopError::ExtraToken { token } => ParseError {
                error: ParseErrorType::ExtraToken(token.1),
                location: token.0,
            },
            LalrpopError::User { error } => ParseError {
                error: ParseErrorType::Lexical(error.error),
                location: error.location,
            },
            LalrpopError::UnrecognizedToken { token, expected } => {
                match token {
                    Some(tok) => ParseError {
                        error: ParseErrorType::UnrecognizedToken(tok.1, expected),
                        location: tok.0,
                    },
                    // EOF was observed when it was unexpected
                    None => ParseError {
                        error: ParseErrorType::EOF,
                        location: Default::default(),
                    },
                }
            }
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} at {}", self.error, self.location)
    }
}

impl fmt::Display for ParseErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ParseErrorType::EOF => write!(f, "Got unexpected EOF"),
            ParseErrorType::ExtraToken(ref tok) => write!(f, "Got extraneous token: {:?}", tok),
            ParseErrorType::InvalidToken => write!(f, "Got invalid token"),
            ParseErrorType::UnrecognizedToken(ref tok, _) => {
                write!(f, "Got unexpected token {}", tok)
            }
            ParseErrorType::Lexical(ref error) => write!(f, "{}", error),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
