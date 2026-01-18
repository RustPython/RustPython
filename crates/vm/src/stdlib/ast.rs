//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

mod pyast;

use crate::builtins::{PyInt, PyStr};
use crate::stdlib::ast::module::{Mod, ModInteractive};
use crate::stdlib::ast::node::BoxedSlice;
use crate::stdlib::ast::python::_ast;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
    TryFromObject, VirtualMachine,
    builtins::PyIntRef,
    builtins::{PyDict, PyModule, PyStrRef, PyType},
    class::{PyClassImpl, StaticType},
    compiler::{CompileError, ParseError},
    convert::ToPyObject,
};
use node::Node;
use ruff_python_ast as ast;
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustpython_compiler_core::{
    LineIndex, OneIndexed, PositionEncoding, SourceFile, SourceFileBuilder, SourceLocation,
};

#[cfg(feature = "parser")]
use ruff_python_parser as parser;

#[cfg(feature = "codegen")]
use rustpython_codegen as codegen;

pub(crate) use python::_ast::NodeAst;

mod python;

mod argument;
mod basic;
mod constant;
mod elif_else_clause;
mod exception;
mod expression;
mod module;
mod node;
mod operator;
mod other;
mod parameter;
mod pattern;
mod statement;
mod string;
mod type_ignore;
mod type_parameters;

fn get_node_field(vm: &VirtualMachine, obj: &PyObject, field: &'static str, typ: &str) -> PyResult {
    vm.get_attribute_opt(obj.to_owned(), field)?
        .ok_or_else(|| vm.new_type_error(format!(r#"required field "{field}" missing from {typ}"#)))
}

fn get_node_field_opt(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
) -> PyResult<Option<PyObjectRef>> {
    Ok(vm
        .get_attribute_opt(obj.to_owned(), field)?
        .filter(|obj| !vm.is_none(obj)))
}

fn get_int_field(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<PyRefExact<PyInt>> {
    get_node_field(vm, obj, field, typ)?
        .downcast_exact(vm)
        .map_err(|_| vm.new_type_error(format!(r#"field "{field}" must have integer type"#)))
}

struct PySourceRange {
    start: PySourceLocation,
    end: PySourceLocation,
}

pub struct PySourceLocation {
    row: Row,
    column: Column,
}

impl PySourceLocation {
    const fn to_source_location(&self) -> SourceLocation {
        SourceLocation {
            line: self.row.get_one_indexed(),
            character_offset: self.column.get_one_indexed(),
        }
    }
}

/// A one-based index into the lines.
#[derive(Clone, Copy)]
struct Row(OneIndexed);

impl Row {
    const fn get(self) -> usize {
        self.0.get()
    }

    const fn get_one_indexed(self) -> OneIndexed {
        self.0
    }
}

/// An UTF-8 index into the line.
#[derive(Clone, Copy)]
struct Column(TextSize);

impl Column {
    const fn get(self) -> usize {
        self.0.to_usize()
    }

    const fn get_one_indexed(self) -> OneIndexed {
        OneIndexed::from_zero_indexed(self.get())
    }
}

fn text_range_to_source_range(source_file: &SourceFile, text_range: TextRange) -> PySourceRange {
    let index = LineIndex::from_source_text(source_file.clone().source_text());
    let source = &source_file.source_text();

    if source.is_empty() {
        return PySourceRange {
            start: PySourceLocation {
                row: Row(OneIndexed::from_zero_indexed(0)),
                column: Column(TextSize::new(0)),
            },
            end: PySourceLocation {
                row: Row(OneIndexed::from_zero_indexed(0)),
                column: Column(TextSize::new(0)),
            },
        };
    }

    let start_row = index.line_index(text_range.start());
    let end_row = index.line_index(text_range.end());
    let start_col = text_range.start() - index.line_start(start_row, source);
    let end_col = text_range.end() - index.line_start(end_row, source);

    PySourceRange {
        start: PySourceLocation {
            row: Row(start_row),
            column: Column(start_col),
        },
        end: PySourceLocation {
            row: Row(end_row),
            column: Column(end_col),
        },
    }
}

fn get_opt_int_field(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
) -> PyResult<Option<PyRefExact<PyInt>>> {
    match get_node_field_opt(vm, obj, field)? {
        Some(val) => val
            .downcast_exact(vm)
            .map(Some)
            .map_err(|_| vm.new_type_error(format!(r#"field "{field}" must have integer type"#))),
        None => Ok(None),
    }
}

fn range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    name: &str,
) -> PyResult<TextRange> {
    let start_row = get_int_field(vm, &object, "lineno", name)?;
    let start_column = get_int_field(vm, &object, "col_offset", name)?;
    // end_lineno and end_col_offset are optional, default to start values
    let end_row =
        get_opt_int_field(vm, &object, "end_lineno")?.unwrap_or_else(|| start_row.clone());
    let end_column =
        get_opt_int_field(vm, &object, "end_col_offset")?.unwrap_or_else(|| start_column.clone());

    // lineno=0 or negative values as a special case (no location info).
    // Use default values (line 1, col 0) when lineno <= 0.
    let start_row_val: i32 = start_row.try_to_primitive(vm)?;
    let end_row_val: i32 = end_row.try_to_primitive(vm)?;
    let start_col_val: i32 = start_column.try_to_primitive(vm)?;
    let end_col_val: i32 = end_column.try_to_primitive(vm)?;

    let location = PySourceRange {
        start: PySourceLocation {
            row: Row(if start_row_val > 0 {
                OneIndexed::new(start_row_val as usize).unwrap_or(OneIndexed::MIN)
            } else {
                OneIndexed::MIN
            }),
            column: Column(TextSize::new(start_col_val.max(0) as u32)),
        },
        end: PySourceLocation {
            row: Row(if end_row_val > 0 {
                OneIndexed::new(end_row_val as usize).unwrap_or(OneIndexed::MIN)
            } else {
                OneIndexed::MIN
            }),
            column: Column(TextSize::new(end_col_val.max(0) as u32)),
        },
    };

    Ok(source_range_to_text_range(source_file, location))
}

fn source_range_to_text_range(source_file: &SourceFile, location: PySourceRange) -> TextRange {
    let index = LineIndex::from_source_text(source_file.clone().source_text());
    let source = &source_file.source_text();

    if source.is_empty() {
        return TextRange::new(TextSize::new(0), TextSize::new(0));
    }

    let start = index.offset(
        location.start.to_source_location(),
        source,
        PositionEncoding::Utf8,
    );
    let end = index.offset(
        location.end.to_source_location(),
        source,
        PositionEncoding::Utf8,
    );

    TextRange::new(start, end)
}

fn node_add_location(
    dict: &Py<PyDict>,
    range: TextRange,
    vm: &VirtualMachine,
    source_file: &SourceFile,
) {
    let range = text_range_to_source_range(source_file, range);
    dict.set_item("lineno", vm.ctx.new_int(range.start.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "col_offset",
        vm.ctx.new_int(range.start.column.get()).into(),
        vm,
    )
    .unwrap();
    dict.set_item("end_lineno", vm.ctx.new_int(range.end.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "end_col_offset",
        vm.ctx.new_int(range.end.column.get()).into(),
        vm,
    )
    .unwrap();
}

#[cfg(feature = "parser")]
pub(crate) fn parse(
    vm: &VirtualMachine,
    source: &str,
    mode: parser::Mode,
) -> Result<PyObjectRef, CompileError> {
    let source_file = SourceFileBuilder::new("".to_owned(), source.to_owned()).finish();
    let top = parser::parse(source, mode.into())
        .map_err(|parse_error| {
            let range = text_range_to_source_range(&source_file, parse_error.location);
            ParseError {
                error: parse_error.error,
                raw_location: parse_error.location,
                location: range.start.to_source_location(),
                end_location: range.end.to_source_location(),
                source_path: "<unknown>".to_string(),
            }
        })?
        .into_syntax();
    let top = match top {
        ast::Mod::Module(m) => Mod::Module(m),
        ast::Mod::Expression(e) => Mod::Expression(e),
    };
    Ok(top.ast_to_object(vm, &source_file))
}

#[cfg(feature = "codegen")]
pub(crate) fn compile(
    vm: &VirtualMachine,
    object: PyObjectRef,
    filename: &str,
    mode: crate::compiler::Mode,
    optimize: Option<u8>,
) -> PyResult {
    let mut opts = vm.compile_opts();
    if let Some(optimize) = optimize {
        opts.optimize = optimize;
    }

    let source_file = SourceFileBuilder::new(filename.to_owned(), "".to_owned()).finish();
    let ast: Mod = Node::ast_from_object(vm, &source_file, object)?;
    let ast = match ast {
        Mod::Module(m) => ast::Mod::Module(m),
        Mod::Interactive(ModInteractive { range, body }) => ast::Mod::Module(ast::ModModule {
            node_index: Default::default(),
            range,
            body,
        }),
        Mod::Expression(e) => ast::Mod::Expression(e),
        Mod::FunctionType(_) => todo!(),
    };
    // TODO: create a textual representation of the ast
    let text = "";
    let source_file = SourceFileBuilder::new(filename, text).finish();
    let code = codegen::compile::compile_top(ast, source_file, mode, opts)
        .map_err(|err| vm.new_syntax_error(&err.into(), None))?; // FIXME source
    Ok(vm.ctx.new_code(code).into())
}

// Used by builtins::compile()
pub const PY_COMPILE_FLAG_AST_ONLY: i32 = 0x0400;

// The following flags match the values from Include/cpython/compile.h
// Caveat emptor: These flags are undocumented on purpose and depending
// on their effect outside the standard library is **unsupported**.
pub const PY_CF_DONT_IMPLY_DEDENT: i32 = 0x200;
pub const PY_CF_ALLOW_INCOMPLETE_INPUT: i32 = 0x4000;
pub const PY_CF_OPTIMIZED_AST: i32 = 0x8000 | PY_COMPILE_FLAG_AST_ONLY;
pub const PY_CF_TYPE_COMMENTS: i32 = 0x1000;

// __future__ flags - sync with Lib/__future__.py
// TODO: These flags aren't being used in rust code
//       CO_FUTURE_ANNOTATIONS does make a difference in the codegen,
//       so it should be used in compile().
//       see compiler/codegen/src/compile.rs
const CO_NESTED: i32 = 0x0010;
const CO_GENERATOR_ALLOWED: i32 = 0;
const CO_FUTURE_DIVISION: i32 = 0x20000;
const CO_FUTURE_ABSOLUTE_IMPORT: i32 = 0x40000;
const CO_FUTURE_WITH_STATEMENT: i32 = 0x80000;
const CO_FUTURE_PRINT_FUNCTION: i32 = 0x100000;
const CO_FUTURE_UNICODE_LITERALS: i32 = 0x200000;
const CO_FUTURE_BARRY_AS_BDFL: i32 = 0x400000;
const CO_FUTURE_GENERATOR_STOP: i32 = 0x800000;
const CO_FUTURE_ANNOTATIONS: i32 = 0x1000000;

// Used by builtins::compile() - the summary of all flags
pub const PY_COMPILE_FLAGS_MASK: i32 = PY_COMPILE_FLAG_AST_ONLY
    | PY_CF_DONT_IMPLY_DEDENT
    | PY_CF_ALLOW_INCOMPLETE_INPUT
    | PY_CF_OPTIMIZED_AST
    | PY_CF_TYPE_COMMENTS
    | CO_NESTED
    | CO_GENERATOR_ALLOWED
    | CO_FUTURE_DIVISION
    | CO_FUTURE_ABSOLUTE_IMPORT
    | CO_FUTURE_WITH_STATEMENT
    | CO_FUTURE_PRINT_FUNCTION
    | CO_FUTURE_UNICODE_LITERALS
    | CO_FUTURE_BARRY_AS_BDFL
    | CO_FUTURE_GENERATOR_STOP
    | CO_FUTURE_ANNOTATIONS;

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _ast::make_module(vm);
    pyast::extend_module_nodes(vm, &module);
    module
}
