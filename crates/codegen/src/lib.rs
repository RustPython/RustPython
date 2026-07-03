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

#[cfg(test)]
pub(crate) fn constant_data_to_ast_constant_value(value: ConstantData) -> ast::ConstantValue {
    match value {
        ConstantData::None => ast::ConstantValue::None,
        ConstantData::Boolean { value } => ast::ConstantValue::Boolean(value),
        ConstantData::Str { value } => ast::ConstantValue::Str(value.to_string().into_boxed_str()),
        ConstantData::Bytes { value } => ast::ConstantValue::Bytes(value.into_boxed_slice()),
        ConstantData::Integer { value } => ast::ConstantValue::Integer(value.to_string().into()),
        ConstantData::Tuple { elements } => ast::ConstantValue::Tuple(
            elements
                .into_iter()
                .map(constant_data_to_ast_constant_value)
                .collect(),
        ),
        ConstantData::Frozenset { elements } => ast::ConstantValue::Frozenset(
            elements
                .into_iter()
                .map(constant_data_to_ast_constant_value)
                .collect(),
        ),
        ConstantData::Float { value } => ast::ConstantValue::Float(value),
        ConstantData::Complex { value } => ast::ConstantValue::Complex {
            real: value.re,
            imag: value.im,
        },
        ConstantData::Ellipsis => ast::ConstantValue::Ellipsis,
        ConstantData::Code { .. } | ConstantData::Slice { .. } => {
            unreachable!("ast.Constant values cannot contain code objects or slices")
        }
    }
}

pub(crate) fn ast_constant_value_to_constant_data(value: ast::ConstantValue) -> ConstantData {
    match value {
        ast::ConstantValue::None => ConstantData::None,
        ast::ConstantValue::Boolean(value) => ConstantData::Boolean { value },
        ast::ConstantValue::Str(value) => ConstantData::Str {
            value: value.to_string().into(),
        },
        ast::ConstantValue::Bytes(value) => ConstantData::Bytes {
            value: value.into_vec(),
        },
        ast::ConstantValue::Integer(value) => ConstantData::Integer {
            value: value
                .parse()
                .expect("RustPython ast.Constant integer values are decimal integers"),
        },
        ast::ConstantValue::Tuple(elements) => ConstantData::Tuple {
            elements: elements
                .into_iter()
                .map(ast_constant_value_to_constant_data)
                .collect(),
        },
        ast::ConstantValue::Frozenset(elements) => ConstantData::Frozenset {
            elements: elements
                .into_iter()
                .map(ast_constant_value_to_constant_data)
                .collect(),
        },
        ast::ConstantValue::Float(value) => ConstantData::Float { value },
        ast::ConstantValue::Complex { real, imag } => ConstantData::Complex {
            value: num_complex::Complex::new(real, imag),
        },
        ast::ConstantValue::Ellipsis => ConstantData::Ellipsis,
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
            Self::Constant(expr) => match &expr.value {
                ast::ConstantValue::None => "None",
                ast::ConstantValue::Boolean(true) => "True",
                ast::ConstantValue::Boolean(false) => "False",
                ast::ConstantValue::Ellipsis => "ellipsis",
                ast::ConstantValue::Tuple(_) => "tuple",
                ast::ConstantValue::Frozenset(_) => "literal",
                ast::ConstantValue::Str(_)
                | ast::ConstantValue::Bytes(_)
                | ast::ConstantValue::Integer(_)
                | ast::ConstantValue::Float(_)
                | ast::ConstantValue::Complex { .. } => "literal",
            },
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
