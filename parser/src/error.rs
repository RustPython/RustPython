//! Define internal parse error types
//! The goal is to provide a matching and a safe error API, maksing errors from LALR
extern crate lalrpop_util;
use self::lalrpop_util::ParseError as InnerError;

use lexer::{LexicalError, Location};
use token::Tok;

use std::error::Error;
use std::fmt;

// A token of type `Tok` was observed, with a span given by the two Location values
type TokSpan = (Location, Tok, Location);

/// Represents an error during parsing
#[derive(Debug, PartialEq)]
pub enum ParseError {
    /// Parser encountered an unexpected end of input
    EOF(Option<Location>),
    /// Parser encountered an extra token
    ExtraToken(TokSpan),
    /// Parser encountered an invalid token
    InvalidToken(Location),
    /// Parser encountered an unexpected token
    UnrecognizedToken(TokSpan, Vec<String>),
    /// Maps to `User` type from `lalrpop-util`
    Other,
}

/// Convert `lalrpop_util::ParseError` to our internal type
impl From<InnerError<Location, Tok, LexicalError>> for ParseError {
    fn from(err: InnerError<Location, Tok, LexicalError>) -> Self {
        match err {
            // TODO: Are there cases where this isn't an EOF?
            InnerError::InvalidToken { location } => ParseError::EOF(Some(location)),
            InnerError::ExtraToken { token } => ParseError::ExtraToken(token),
            // Inner field is a unit-like enum `LexicalError::StringError` with no useful info
            InnerError::User { .. } => ParseError::Other,
            InnerError::UnrecognizedToken { token, expected } => {
                match token {
                    Some(tok) => ParseError::UnrecognizedToken(tok, expected),
                    // EOF was observed when it was unexpected
                    None => ParseError::EOF(None),
                }
            }
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ParseError::EOF(ref location) => {
                if let Some(l) = location {
                    write!(f, "Got unexpected EOF at: {:?}", l)
                } else {
                    write!(f, "Got unexpected EOF")
                }
            }
            ParseError::ExtraToken(ref t_span) => {
                write!(f, "Got extraneous token: {:?} at: {:?}", t_span.1, t_span.0)
            }
            ParseError::InvalidToken(ref location) => {
                write!(f, "Got invalid token at: {:?}", location)
            }
            ParseError::UnrecognizedToken(ref t_span, _) => {
                write!(f, "Got unexpected token: {:?} at {:?}", t_span.1, t_span.0)
            }
            // This is user defined, it probably means a more useful error should have been given upstream.
            ParseError::Other => write!(f, "Got unsupported token(s)"),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}
