//! Compile a Python AST or source code into bytecode consumable by RustPython.
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-compiler/")]

#[macro_use]
extern crate log;

type IndexMap<K, V> = indexmap::IndexMap<K, V, ahash::RandomState>;
type IndexSet<T> = indexmap::IndexSet<T, ahash::RandomState>;

pub mod compile;
pub mod error;
pub mod ir;
mod string_parser;
pub mod symboltable;

pub use compile::CompileOpts;
use ruff_python_ast::Expr;

pub(crate) use compile::InternalResult;

pub trait ToPythonName {
    /// Returns a short name for the node suitable for use in error messages.
    fn python_name(&self) -> &'static str;
}

impl ToPythonName for Expr {
    fn python_name(&self) -> &'static str {
        match self {
            Self::BoolOp { .. } | Self::BinOp { .. } | Self::UnaryOp { .. } => "operator",
            Self::Subscript { .. } => "subscript",
            Self::Await { .. } => "await expression",
            Self::Yield { .. } | Self::YieldFrom { .. } => "yield expression",
            Self::Compare { .. } => "comparison",
            Self::Attribute { .. } => "attribute",
            Self::Call { .. } => "function call",
            Self::BooleanLiteral(b) => {
                if b.value {
                    "True"
                } else {
                    "False"
                }
            }
            Self::EllipsisLiteral(_) => "ellipsis",
            Self::NoneLiteral(_) => "None",
            Self::NumberLiteral(_) | Self::BytesLiteral(_) | Self::StringLiteral(_) => "literal",
            Self::Tuple(_) => "tuple",
            Self::List { .. } => "list",
            Self::Dict { .. } => "dict display",
            Self::Set { .. } => "set display",
            Self::ListComp { .. } => "list comprehension",
            Self::DictComp { .. } => "dict comprehension",
            Self::SetComp { .. } => "set comprehension",
            Self::Generator { .. } => "generator expression",
            Self::Starred { .. } => "starred",
            Self::Slice { .. } => "slice",
            Self::FString { .. } => "f-string expression",
            Self::TString { .. } => "t-string expression",
            Self::Name { .. } => "name",
            Self::Lambda { .. } => "lambda",
            Self::If { .. } => "conditional expression",
            Self::Named { .. } => "named expression",
            Self::IpyEscapeCommand(_) => todo!(),
        }
    }
}
