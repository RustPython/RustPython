//! Compile a Python AST or source code into bytecode consumable by RustPython.
#![cfg_attr(not(feature = "std"), no_std)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/RustPython/RustPython/main/logo.png")]
#![doc(html_root_url = "https://docs.rs/rustpython-compiler/")]

#[macro_use]
extern crate log;

extern crate alloc;

use alloc::{string::String, vec::Vec};
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
use ruff_text_size::{TextRange, TextSize};
use rustpython_compiler_core::SourceFile;
use rustpython_wtf8::Wtf8Buf;

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

/// The value of a string literal.
///
/// The parser has nowhere to keep a lone surrogate in the value it hands over,
/// and leaves a replacement character standing in for one, so a literal that
/// carries any is read again from the source it was written in.
#[must_use]
pub fn string_literal_value(source_file: &SourceFile, string: &ast::StringLiteralValue) -> Wtf8Buf {
    let value = string.to_str();
    if value.contains(char::REPLACEMENT_CHARACTER) {
        string
            .iter()
            .map(|part| string_literal_part_value(source_file, part))
            .collect()
    } else {
        value.into()
    }
}

/// The value of one of the literals a string literal is written as.
#[must_use]
pub fn string_literal_part_value(source_file: &SourceFile, string: &ast::StringLiteral) -> Wtf8Buf {
    if string.value.contains(char::REPLACEMENT_CHARACTER) {
        let source = source_file.slice(string.range);
        string_parser::parse_string_literal(source, string.flags.into()).into()
    } else {
        string.value.to_string().into()
    }
}

/// The value of a literal written between the interpolations of an f-string or
/// a t-string.
#[must_use]
pub fn interpolated_string_literal_value(
    source_file: &SourceFile,
    element: &ast::InterpolatedStringLiteralElement,
    flags: ast::AnyStringFlags,
) -> Wtf8Buf {
    if element.value.contains(char::REPLACEMENT_CHARACTER) {
        let source = source_file.slice(element.range);
        string_parser::parse_fstring_literal_element(source.into(), flags).into()
    } else {
        element.value.to_string().into()
    }
}

/// The text a `{expr=}` interpolation puts in front of its value, and the range
/// that text covers.
///
/// The text is the expression as it is written plus whatever the parser found
/// around it inside the braces, with comments taken out. The range covers that
/// surrounding text as it stands, comments included.
#[must_use]
pub fn interpolation_debug_text(
    source_file: &SourceFile,
    debug_text: &ast::DebugText,
    expression_range: TextRange,
) -> (String, TextRange) {
    let leading = debug_text.leading.as_str();
    let trailing = debug_text.trailing.as_str();
    let text = [
        strip_python_comments(leading).as_str(),
        source_file.slice(expression_range),
        strip_python_comments(trailing).as_str(),
    ]
    .concat();
    let width =
        |len: usize| TextSize::new(u32::try_from(len).expect("debug interpolation text too long"));
    let range = TextRange::new(
        expression_range.start() - width(leading.len()),
        expression_range.end() + width(trailing.len()),
    );
    (text, range)
}

fn strip_python_comments(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(text.len());
    let mut quote = None;
    let mut triple_quoted = false;
    let mut escaped = false;
    let mut in_comment = false;
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if matches!(ch, '\n' | '\r') {
                in_comment = false;
                result.push(ch);
            }
            index += 1;
            continue;
        }

        if let Some(delimiter) = quote {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if triple_quoted
                && ch == delimiter
                && chars.get(index + 1) == Some(&delimiter)
                && chars.get(index + 2) == Some(&delimiter)
            {
                result.push(delimiter);
                result.push(delimiter);
                quote = None;
                index += 2;
            } else if !triple_quoted && ch == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }

        match ch {
            '#' => in_comment = true,
            '\'' | '"' => {
                quote = Some(ch);
                triple_quoted =
                    chars.get(index + 1) == Some(&ch) && chars.get(index + 2) == Some(&ch);
                result.push(ch);
                if triple_quoted {
                    result.push(ch);
                    result.push(ch);
                    index += 2;
                }
            }
            _ => result.push(ch),
        }
        index += 1;
    }
    result
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
