//! Define internal parse error types
//! The goal is to provide a matching and a safe error API, maksing errors from LALR
extern crate lalrpop_util;
use self::lalrpop_util::ParseError as InnerError;

use crate::lexer::{LexicalError, LexicalErrorType, Location};
use crate::token::Tok;

use std::error::Error;
use std::fmt;

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
impl From<InnerError<Location, Tok, LexicalError>> for ParseError {
    fn from(err: InnerError<Location, Tok, LexicalError>) -> Self {
        match err {
            // TODO: Are there cases where this isn't an EOF?
            InnerError::InvalidToken { location } => ParseError {
                error: ParseErrorType::EOF,
                location,
            },
            InnerError::ExtraToken { token } => ParseError {
                error: ParseErrorType::ExtraToken(token.1),
                location: token.0,
            },
            InnerError::User { error } => ParseError {
                error: ParseErrorType::Lexical(error.error),
                location: error.location,
            },
            InnerError::UnrecognizedToken { token, expected } => {
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
                write!(f, "Got unexpected token: {:?}", tok)
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
