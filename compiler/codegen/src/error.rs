use std::fmt;

use ruff_python_ast::Expr;
use ruff_text_size::TextRange;

// pub type CodegenError = rustpython_parser_core::source_code::LocatedError<CodegenErrorType>;

pub struct CodegenError {
    pub range: Option<TextRange>,
    pub error: CodegenErrorType,
    pub source_path: String,
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
    NotImplementedYet, // RustPython marker for unimplemented features
}

impl std::error::Error for CodegenErrorType {}

impl fmt::Display for CodegenErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
            NotImplementedYet => {
                write!(f, "RustPython does not implement this feature yet")
            }
        }
    }
}

pub trait ToPythonName {
    /// Returns a short name for the node suitable for use in error messages.
    fn python_name(&self) -> &'static str;
}

impl ToPythonName for Expr {
    fn python_name(&self) -> &'static str {
        match self {
            Expr::BoolOp { .. } | Expr::BinOp { .. } | Expr::UnaryOp { .. } => "operator",
            Expr::Subscript { .. } => "subscript",
            Expr::Await { .. } => "await expression",
            Expr::Yield { .. } | Expr::YieldFrom { .. } => "yield expression",
            Expr::Compare { .. } => "comparison",
            Expr::Attribute { .. } => "attribute",
            Expr::Call { .. } => "function call",
            Expr::BooleanLiteral(b) => {
                if b.value {
                    "True"
                } else {
                    "False"
                }
            }
            Expr::EllipsisLiteral(_) => "ellipsis",
            Expr::NoneLiteral(_) => "None",
            Expr::NumberLiteral(_) | Expr::BytesLiteral(_) | Expr::StringLiteral(_) => "literal",
            Expr::Tuple(_) => "tuple",
            Expr::List { .. } => "list",
            Expr::Dict { .. } => "dict display",
            Expr::Set { .. } => "set display",
            Expr::ListComp { .. } => "list comprehension",
            Expr::DictComp { .. } => "dict comprehension",
            Expr::SetComp { .. } => "set comprehension",
            Expr::Generator { .. } => "generator expression",
            Expr::Starred { .. } => "starred",
            Expr::Slice { .. } => "slice",
            Expr::FString { .. } => "f-string expression",
            Expr::Name { .. } => "name",
            Expr::Lambda { .. } => "lambda",
            Expr::If { .. } => "conditional expression",
            Expr::Named { .. } => "named expression",
            Expr::IpyEscapeCommand(_) => todo!(),
        }
    }
}
