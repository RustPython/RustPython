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
pub mod symboltable;

pub use compile::CompileOpts;
use ruff_python_ast::Expr;

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
