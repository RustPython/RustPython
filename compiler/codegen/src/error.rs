use rustpython_compiler_core::SourceLocation;
use std::fmt::{self, Display};
use thiserror::Error;

#[derive(Debug)]
pub enum PatternUnreachableReason {
    NameCapture,
    Wildcard,
}

impl Display for PatternUnreachableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NameCapture => write!(f, "name capture"),
            Self::Wildcard => write!(f, "wildcard"),
        }
    }
}

// pub type CodegenError = rustpython_parser_core::source_code::LocatedError<CodegenErrorType>;

#[derive(Error, Debug)]
pub struct CodegenError {
    pub location: Option<SourceLocation>,
    #[source]
    pub error: CodegenErrorType,
    pub source_path: String,
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO:
        self.error.fmt(f)
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum InternalError {
    StackOverflow,
    StackUnderflow,
    MissingSymbol(String),
}

impl Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StackOverflow => write!(f, "stack overflow"),
            Self::StackUnderflow => write!(f, "stack underflow"),
            Self::MissingSymbol(s) => write!(
                f,
                "The symbol '{s}' must be present in the symbol table, even when it is undefined in python."
            ),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum CodegenErrorType {
    /// Invalid assignment, cannot store value in target.
    Assign(&'static str),
    /// Invalid delete
    Delete(&'static str),
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
    FunctionImportStar,
    TooManyStarUnpack,
    EmptyWithItems,
    EmptyWithBody,
    ForbiddenName,
    DuplicateStore(String),
    UnreachablePattern(PatternUnreachableReason),
    RepeatedAttributePattern,
    ConflictingNameBindPattern,
    NotImplementedYet, // RustPython marker for unimplemented features
}

impl std::error::Error for CodegenErrorType {}

impl fmt::Display for CodegenErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use CodegenErrorType::*;
        match self {
            Assign(target) => write!(f, "cannot assign to {target}"),
            Delete(target) => write!(f, "cannot delete {target}"),
            SyntaxError(err) => write!(f, "{}", err.as_str()),
            MultipleStarArgs => {
                write!(f, "two starred expressions in assignment")
            }
            InvalidStarExpr => write!(f, "cannot use starred expression here"),
            InvalidBreak => write!(f, "'break' outside loop"),
            InvalidContinue => write!(f, "'continue' outside loop"),
            InvalidReturn => write!(f, "'return' outside function"),
            InvalidYield => write!(f, "'yield' outside function"),
            InvalidYieldFrom => write!(f, "'yield from' outside function"),
            InvalidAwait => write!(f, "'await' outside async function"),
            AsyncYieldFrom => write!(f, "'yield from' inside async function"),
            AsyncReturnValue => {
                write!(f, "'return' with value inside async generator")
            }
            InvalidFuturePlacement => write!(
                f,
                "from __future__ imports must occur at the beginning of the file"
            ),
            InvalidFutureFeature(feat) => {
                write!(f, "future feature {feat} is not defined")
            }
            FunctionImportStar => {
                write!(f, "import * only allowed at module level")
            }
            TooManyStarUnpack => {
                write!(f, "too many expressions in star-unpacking assignment")
            }
            EmptyWithItems => {
                write!(f, "empty items on With")
            }
            EmptyWithBody => {
                write!(f, "empty body on With")
            }
            ForbiddenName => {
                write!(f, "forbidden attribute name")
            }
            DuplicateStore(s) => {
                write!(f, "duplicate store {s}")
            }
            UnreachablePattern(reason) => {
                write!(f, "{reason} makes remaining patterns unreachable")
            }
            RepeatedAttributePattern => {
                write!(f, "attribute name repeated in class pattern")
            }
            ConflictingNameBindPattern => {
                write!(f, "alternative patterns bind different names")
            }
            NotImplementedYet => {
                write!(f, "RustPython does not implement this feature yet")
            }
        }
    }
}
