use std::fmt;

pub type CodegenError = rustpython_compiler_core::BaseError<CodegenErrorType>;

#[derive(Debug, thiserror::Error)]
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
    NotImplementedYet, // RustPython marker for unimplemented features
}

impl fmt::Display for CodegenErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use CodegenErrorType::*;
        match self {
            Assign(target) => write!(f, "cannot assign to {}", target),
            Delete(target) => write!(f, "cannot delete {}", target),
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
                write!(f, "future feature {} is not defined", feat)
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
            NotImplementedYet => {
                write!(f, "RustPython does not implement this feature yet")
            }
        }
    }
}
