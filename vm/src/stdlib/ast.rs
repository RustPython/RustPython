//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

mod gen;

use crate::builtins::{PyInt, PyStr};
use crate::stdlib::ast::module::{Mod, ModInteractive};
use crate::stdlib::ast::node::BoxedSlice;
use crate::stdlib::ast::python::_ast;
use crate::{
    builtins::PyIntRef,
    builtins::{PyDict, PyModule, PyStrRef, PyType},
    class::{PyClassImpl, StaticType},
    compiler::core::bytecode::OpArgType,
    compiler::{CompileError, ParseError},
    convert::ToPyObject,
    source::SourceCode,
    source::SourceLocation,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
    TryFromObject, VirtualMachine,
};
use node::Node;
use ruff_python_ast as ruff;
use ruff_text_size::{Ranged, TextRange, TextSize};

#[cfg(feature = "parser")]
use ruff_python_parser as parser;
#[cfg(feature = "codegen")]
use rustpython_codegen as codegen;

pub(crate) use python::_ast::NodeAst;

mod python;

mod argument;
mod basic;
mod elif_else_clause;
mod exception;
mod expression;
mod module;
mod operator;
mod other;
mod parameter;
mod pattern;
mod statement;
mod type_ignore;
mod type_parameters;

fn get_node_field(vm: &VirtualMachine, obj: &PyObject, field: &'static str, typ: &str) -> PyResult {
    vm.get_attribute_opt(obj.to_owned(), field)?
        .ok_or_else(|| vm.new_type_error(format!("required field \"{field}\" missing from {typ}")))
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
) -> PyResult<Option<PyRefExact<PyInt>>> {
    Ok(get_node_field_opt(vm, &obj, field)?
        .map(|obj| obj.downcast_exact(vm))
        .transpose()
        .unwrap())
}

mod node;

struct SourceRange {
    start: SourceLocation,
    end: SourceLocation,
}

fn source_location_to_text_size(source_location: SourceLocation) -> TextSize {
    // TODO: Maybe implement this?
    TextSize::default()
}

fn text_range_to_source_range(text_range: TextRange) -> SourceRange {
    // TODO: Maybe implement this?
    SourceRange {
        start: SourceLocation::default(),
        end: SourceLocation::default(),
    }
}

fn range_from_object(vm: &VirtualMachine, object: PyObjectRef, name: &str) -> PyResult<TextRange> {
    fn make_location(row: PyIntRef, column: PyIntRef) -> Option<SourceLocation> {
        // TODO: Maybe implement this?
        // let row = row.to_u64().unwrap().try_into().unwrap();
        // let column = column.to_u64().unwrap().try_into().unwrap();
        // Some(SourceLocation {
        //     row: LineNumber::new(row)?,
        //     column: LineNumber::from_zero_indexed(column),
        // })

        None
    }

    let row = get_node_field(vm, &object, "lineno", name)?;
    let row = row.downcast_exact::<PyInt>(vm).unwrap().into_pyref();
    let column = get_node_field(vm, &object, "col_offset", name)?;
    let column = column.downcast_exact::<PyInt>(vm).unwrap().into_pyref();
    let location = make_location(row, column);
    let end_row = get_int_field(vm, &object, "end_lineno")?;
    let end_column = get_int_field(vm, &object, "end_col_offset")?;
    let end_location = if let (Some(row), Some(column)) = (end_row, end_column) {
        make_location(row.into_pyref(), column.into_pyref())
    } else {
        None
    };
    let range = TextRange::new(
        source_location_to_text_size(location.unwrap_or_default()),
        source_location_to_text_size(end_location.unwrap_or_default()),
    );
    Ok(range)
}

fn node_add_location(dict: &Py<PyDict>, range: TextRange, vm: &VirtualMachine) {
    let range = text_range_to_source_range(range);
    dict.set_item("lineno", vm.ctx.new_int(range.start.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "col_offset",
        vm.ctx.new_int(range.start.column.to_zero_indexed()).into(),
        vm,
    )
    .unwrap();
    dict.set_item("end_lineno", vm.ctx.new_int(range.end.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "end_col_offset",
        vm.ctx.new_int(range.end.column.to_zero_indexed()).into(),
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
    let top = parser::parse(source, mode)
        .map_err(|parse_error| ParseError {
            error: parse_error.error,
            location: text_range_to_source_range(parse_error.location).start,
            source_path: "<unknown>".to_string(),
        })?
        .into_syntax();
    let top = match top {
        ruff::Mod::Module(m) => Mod::Module(m),
        ruff::Mod::Expression(e) => Mod::Expression(e),
    };
    Ok(top.ast_to_object(vm))
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

    let ast: Mod = Node::ast_from_object(vm, object)?;
    let ast = match ast {
        Mod::Module(m) => ruff::Mod::Module(m),
        Mod::Interactive(ModInteractive { range, body }) => {
            ruff::Mod::Module(ruff::ModModule { range, body })
        }
        Mod::Expression(e) => ruff::Mod::Expression(e),
        Mod::FunctionType(_) => todo!(),
    };
    // TODO: create a textual representation of the ast
    let text = "";
    let source_code = SourceCode::new(filename, text);
    let code = codegen::compile::compile_top(ast, source_code, mode, opts)
        .map_err(|err| vm.new_syntax_error(&err.into(), None))?; // FIXME source
    Ok(vm.ctx.new_code(code).into())
}

// Used by builtins::compile()
pub const PY_COMPILE_FLAG_AST_ONLY: i32 = 0x0400;

// The following flags match the values from Include/cpython/compile.h
// Caveat emptor: These flags are undocumented on purpose and depending
// on their effect outside the standard library is **unsupported**.
const PY_CF_DONT_IMPLY_DEDENT: i32 = 0x200;
const PY_CF_ALLOW_INCOMPLETE_INPUT: i32 = 0x4000;

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
    gen::extend_module_nodes(vm, &module);
    module
}
