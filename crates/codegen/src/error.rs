use alloc::fmt;
use core::fmt::Display;
use rustpython_compiler_core::SourceLocation;
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
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
    StackUnderflow,
    InconsistentStackDepth,
    InvalidStackEffect,
    MalformedControlFlowGraph,
    MissingSymbol(String),
}

impl Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StackUnderflow => write!(f, "Invalid CFG, stack underflow"),
            Self::InconsistentStackDepth => write!(f, "Invalid CFG, inconsistent stackdepth"),
            Self::InvalidStackEffect => write!(f, "Invalid stack effect"),
            Self::MalformedControlFlowGraph => write!(f, "malformed control flow graph."),
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
    InvalidAsyncFor,
    InvalidAsyncWith,
    InvalidAsyncComprehension,
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
    /// break/continue/return inside except* block
    BreakContinueReturnInExceptStar,
    NotImplementedYet, // RustPython marker for unimplemented features
}

impl core::error::Error for CodegenErrorType {}

impl fmt::Display for CodegenErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assign(target) => write!(f, "cannot assign to {target}"),
            Self::Delete(target) => write!(f, "cannot delete {target}"),
            Self::SyntaxError(err) => write!(f, "{}", err.as_str()),
            Self::MultipleStarArgs => {
                write!(f, "multiple starred expressions in assignment")
            }
            Self::InvalidStarExpr => write!(f, "can't use starred expression here"),
            Self::InvalidBreak => write!(f, "'break' outside loop"),
            Self::InvalidContinue => write!(f, "'continue' not properly in loop"),
            Self::InvalidReturn => write!(f, "'return' outside function"),
            Self::InvalidYield => write!(f, "'yield' outside function"),
            Self::InvalidYieldFrom => write!(f, "'yield from' outside function"),
            Self::InvalidAwait => write!(f, "'await' outside async function"),
            Self::InvalidAsyncFor => write!(f, "'async for' outside async function"),
            Self::InvalidAsyncWith => write!(f, "'async with' outside async function"),
            Self::InvalidAsyncComprehension => {
                write!(
                    f,
                    "asynchronous comprehension outside of an asynchronous function"
                )
            }
            Self::AsyncYieldFrom => write!(f, "'yield from' inside async function"),
            Self::AsyncReturnValue => {
                write!(f, "'return' with value inside async generator")
            }
            Self::InvalidFuturePlacement => write!(
                f,
                "from __future__ imports must occur at the beginning of the file"
            ),
            Self::InvalidFutureFeature(feat) => {
                write!(f, "future feature {feat} is not defined")
            }
            Self::FunctionImportStar => {
                write!(f, "import * only allowed at module level")
            }
            Self::TooManyStarUnpack => {
                write!(f, "too many expressions in star-unpacking assignment")
            }
            Self::EmptyWithItems => {
                write!(f, "empty items on With")
            }
            Self::EmptyWithBody => {
                write!(f, "empty body on With")
            }
            Self::ForbiddenName => {
                write!(f, "forbidden attribute name")
            }
            Self::DuplicateStore(s) => {
                write!(f, "duplicate store {s}")
            }
            Self::UnreachablePattern(reason) => {
                write!(f, "{reason} makes remaining patterns unreachable")
            }
            Self::RepeatedAttributePattern => {
                write!(f, "attribute name repeated in class pattern")
            }
            Self::ConflictingNameBindPattern => {
                write!(f, "alternative patterns bind different names")
            }
            Self::BreakContinueReturnInExceptStar => {
                write!(
                    f,
                    "'break', 'continue' and 'return' cannot appear in an except* block"
                )
            }
            Self::NotImplementedYet => {
                write!(f, "RustPython does not implement this feature yet")
            }
        }
    }
}
