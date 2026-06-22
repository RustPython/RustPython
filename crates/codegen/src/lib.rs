//! Compile a Python AST or source code into bytecode consumable by RustPython.
#![cfg_attr(not(feature = "std"), no_std)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-compiler/")]

#[macro_use]
extern crate log;

extern crate alloc;

use rustpython_compiler_core::bytecode::ConstantData;

type IndexMap<K, V> = indexmap::IndexMap<K, V, rapidhash::quality::RandomState>;
type IndexSet<T> = indexmap::IndexSet<T, rapidhash::quality::RandomState>;

pub mod compile;
pub mod error;
pub mod ir;
pub mod preprocess;
mod string_parser;
pub mod symboltable;
mod unparse;

pub use compile::CompileOpts;
use ruff_python_ast as ast;

pub(crate) use compile::InternalResult;

#[derive(Clone, Debug)]
pub struct PublicAstInterpolation {
    pub str: ConstantData,
    pub format_spec: Option<Box<ast::Expr>>,
}

#[derive(Clone, Debug)]
pub struct PublicAstFormattedValue {
    pub format_spec: Option<Box<ast::Expr>>,
}

#[derive(Clone, Debug)]
pub struct PublicAstExprList {
    pub values: Vec<ast::Expr>,
}

/// Dense side table keyed by public-AST `NodeIndex`.
///
/// Public `_ast` constructors allocate synthetic node indexes from zero, so a
/// `Vec<Option<T>>` gives O(1) lookup without hashing or insertion-order state.
#[derive(Clone, Debug, Default)]
pub struct PublicAstNodeMap<T> {
    values: Vec<Option<T>>,
}

impl<T> PublicAstNodeMap<T> {
    #[must_use]
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub fn insert(&mut self, index: ast::NodeIndex, value: T) -> Option<T> {
        let index = index
            .as_u32()
            .expect("public AST side table cannot store NodeIndex::NONE")
            as usize;
        if self.values.len() <= index {
            self.values.resize_with(index + 1, || None);
        }
        self.values[index].replace(value)
    }

    #[must_use]
    pub fn get(&self, index: &ast::NodeIndex) -> Option<&T> {
        let index = index.as_u32()? as usize;
        self.values.get(index)?.as_ref()
    }

    pub fn get_mut(&mut self, index: &ast::NodeIndex) -> Option<&mut T> {
        let index = index.as_u32()? as usize;
        self.values.get_mut(index)?.as_mut()
    }

    #[must_use]
    pub fn contains_key(&self, index: &ast::NodeIndex) -> bool {
        self.get(index).is_some()
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.values.iter().filter_map(Option::as_ref)
    }

    pub fn is_empty(&self) -> bool {
        self.values.iter().all(Option::is_none)
    }
}

pub trait ToPythonName {
    /// Returns a short name for the node suitable for use in error messages.
    fn python_name(&self) -> &'static str;
}

impl ToPythonName for ast::Expr {
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
            Self::IpyEscapeCommand(_) => "expression",
        }
    }
}
