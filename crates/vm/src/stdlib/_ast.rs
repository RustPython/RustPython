//! `ast` standard module for abstract syntax trees.

//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

pub(crate) use python::_ast::module_def;

mod pyast;

use crate::builtins::{PyInt, PyStr};
use crate::stdlib::_ast::module::{Mod, ModFunctionType, ModInteractive, ModModule};
use crate::stdlib::_ast::node::BoxedSlice;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
    builtins::{PyDict, PyList, PyModule, PyTuple, PyType, PyUtf8StrRef},
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
mod repr;
mod validate;

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

/// Return the cached singleton instance for an operator/context node type,
/// or create a new instance if none exists.
fn singleton_node_to_object(vm: &VirtualMachine, node_type: &'static Py<PyType>) -> PyObjectRef {
    if let Some(instance) = node_type.get_attr(vm.ctx.intern_str("_instance")) {
        return instance;
    }
    NodeAst
        .into_ref_with_type(vm, node_type.to_owned())
        .unwrap()
        .into()
}

fn is_node_instance(
    vm: &VirtualMachine,
    object: &PyObjectRef,
    node_type: &'static Py<PyType>,
) -> PyResult<bool> {
    object.is_instance(node_type.as_object(), vm)
}

fn is_ast_instance(vm: &VirtualMachine, object: &PyObjectRef) -> PyResult<bool> {
    let ast_type = NodeAst::make_static_type();
    object.is_instance(ast_type.as_object(), vm)
}

fn get_node_field(vm: &VirtualMachine, obj: &PyObject, field: &'static str, typ: &str) -> PyResult {
    vm.get_attribute_opt(obj.to_owned(), field)?
        .ok_or_else(|| vm.new_type_error(format!(r#"required field "{field}" missing from {typ}"#)))
}

/// Read a required scalar field. The generated `obj2ast_*` converters only
/// reject a missing required attribute here; if the field exists but is `None`,
/// the nested converter handles it.
fn get_node_field_required(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult {
    get_node_field(vm, obj, field, typ)
}

fn get_required_identifier_field<T: Node>(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<T> {
    let value = get_node_field_required(vm, obj, field, typ)?;
    if vm.is_none(&value) {
        return Err(vm.new_value_error(format!("field '{field}' is required for {typ}")));
    }
    Node::ast_from_object(vm, source_file, value)
}

fn get_required_node_field<T: Node>(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<T> {
    let value = get_node_field_required(vm, obj, field, typ)?;
    if vm.is_none(&value) {
        return Err(vm.new_value_error(format!("field '{field}' is required for {typ}")));
    }
    let recursion_context = format!(" while traversing '{typ}' node");
    vm.with_recursion(&recursion_context, || {
        Node::ast_from_object(vm, source_file, value)
    })
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

fn get_node_list_field<T: Node>(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<Vec<T>> {
    let value = get_node_list_field_object(vm, obj, field, typ)?;
    let list = value.downcast_ref::<PyList>().unwrap();
    convert_node_list_field(vm, source_file, list, field, typ)
}

fn get_node_list_field_object(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<PyObjectRef> {
    let Some(value) = vm.get_attribute_opt(obj.to_owned(), field)? else {
        return Ok(vm.ctx.new_list(Vec::new()).into());
    };
    value.downcast_ref::<PyList>().ok_or_else(|| {
        vm.new_type_error(format!(
            r#"{typ} field "{field}" must be a list, not a {}"#,
            value.class().name()
        ))
    })?;
    Ok(value)
}

fn convert_node_list_field<T: Node>(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    list: &PyList,
    field: &'static str,
    typ: &str,
) -> PyResult<Vec<T>> {
    let len = list.borrow_vec().len();
    let mut result = Vec::with_capacity(len);
    let recursion_context = format!(" while traversing '{typ}' node");
    for i in 0..len {
        let item = {
            let items = list.borrow_vec();
            if items.len() != len {
                return Err(vm.new_runtime_error(format!(
                    r#"{typ} field "{field}" changed size during iteration"#
                )));
            }
            items[i].clone()
        };
        result.push(vm.with_recursion(&recursion_context, || {
            Node::ast_from_object(vm, source_file, item)
        })?);
        if list.borrow_vec().len() != len {
            return Err(vm.new_runtime_error(format!(
                r#"{typ} field "{field}" changed size during iteration"#
            )));
        }
    }
    Ok(result)
}

fn get_node_boxed_slice_field<T: Node>(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<Box<[T]>> {
    Ok(get_node_list_field(vm, source_file, obj, field, typ)?.into_boxed_slice())
}

fn runtime_expr_list_from_values(
    values: Vec<Option<ast::Expr>>,
) -> (Option<Vec<Option<ast::Expr>>>, Vec<ast::Expr>) {
    let metadata = runtime_expr_list_metadata(&values);
    (metadata, lower_runtime_expr_list(values))
}

fn runtime_expr_boxed_slice_from_values(
    values: Vec<Option<ast::Expr>>,
) -> (Option<Vec<Option<ast::Expr>>>, Box<[ast::Expr]>) {
    let (metadata, values) = runtime_expr_list_from_values(values);
    (metadata, values.into_boxed_slice())
}

fn runtime_expr_list_metadata(values: &[Option<ast::Expr>]) -> Option<Vec<Option<ast::Expr>>> {
    values.iter().any(Option::is_none).then(|| values.to_vec())
}

fn runtime_stmt_list_from_values(
    values: Vec<Option<ast::Stmt>>,
) -> (Option<Vec<Option<ast::Stmt>>>, ast::Suite) {
    let metadata = runtime_stmt_list_metadata(&values);
    (metadata, lower_runtime_stmt_list(values))
}

fn runtime_stmt_list_metadata(values: &[Option<ast::Stmt>]) -> Option<Vec<Option<ast::Stmt>>> {
    values.iter().any(Option::is_none).then(|| values.to_vec())
}

fn runtime_except_handler_list_metadata(
    values: &[Option<ast::ExceptHandler>],
) -> Option<Vec<Option<ast::ExceptHandler>>> {
    values.iter().any(Option::is_none).then(|| values.to_vec())
}

fn lower_runtime_stmt_list(values: Vec<Option<ast::Stmt>>) -> ast::Suite {
    values
        .into_iter()
        .map(|value| value.unwrap_or_else(runtime_null_stmt_placeholder))
        .collect()
}

fn lower_runtime_expr_list(values: Vec<Option<ast::Expr>>) -> Vec<ast::Expr> {
    values
        .into_iter()
        .map(|value| value.unwrap_or_else(runtime_null_expr_placeholder))
        .collect()
}

fn runtime_null_stmt_placeholder() -> ast::Stmt {
    ast::Stmt::Pass(ast::StmtPass {
        range: Default::default(),
        node_index: Default::default(),
    })
}

fn runtime_null_expr_placeholder() -> ast::Expr {
    ast::Expr::NoneLiteral(ast::ExprNoneLiteral {
        range: Default::default(),
        node_index: Default::default(),
    })
}

fn get_int_field(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<i32> {
    node_object_to_i32(vm, get_node_field(vm, obj, field, typ)?)
}

pub(super) fn node_object_to_i32(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<i32> {
    if obj.is(&vm.ctx.true_value) {
        return Ok(1);
    }
    if obj.is(&vm.ctx.false_value) {
        return Ok(0);
    }
    let int: PyRef<PyInt> = match obj.clone().try_into_value(vm) {
        Ok(int) => int,
        Err(_) => {
            return Err(vm.new_value_error(format!("invalid integer value: {}", obj.repr(vm)?)));
        }
    };
    i32::try_from(int.as_bigint())
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C int"))
}

pub(super) fn node_object_to_ast_string(
    vm: &VirtualMachine,
    obj: PyObjectRef,
) -> PyResult<PyObjectRef> {
    let cls = obj.class();
    if cls.is(vm.ctx.types.str_type) || cls.is(vm.ctx.types.bytes_type) {
        Ok(obj)
    } else {
        Err(vm.new_type_error("AST string must be of type str or bytes"))
    }
}

fn get_ast_string_field_opt(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
) -> PyResult<Option<PyObjectRef>> {
    get_node_field_opt(vm, obj, field)?
        .map(|obj| node_object_to_ast_string(vm, obj))
        .transpose()
}

struct PySourceRange {
    start: PySourceLocation,
    end: PySourceLocation,
}

pub(crate) struct PySourceLocation {
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
    let (end_row, end_col) = {
        let end_col = text_range.end() - index.line_start(end_row, source);
        if end_col == TextSize::new(0) && end_row > start_row {
            let prev_line_end = text_range.end() - TextSize::new(1);
            let row = index.line_index(prev_line_end);
            let col = prev_line_end - index.line_start(row, source) + TextSize::new(1);
            (row, col)
        } else {
            (end_row, end_col)
        }
    };

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
) -> PyResult<Option<i32>> {
    match get_node_field_opt(vm, obj, field)? {
        Some(val) => node_object_to_i32(vm, val).map(Some),
        None => Ok(None),
    }
}

fn get_attribute_from_field(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    field: PyObjectRef,
) -> PyResult<Option<PyObjectRef>> {
    let field = field
        .downcast::<PyStr>()
        .map_err(|_| vm.new_type_error("attribute name must be string"))?;
    vm.get_attribute_opt(obj.clone(), &field)
}

#[derive(Default)]
struct AstSourceExtent {
    max_line: usize,
    max_col: usize,
}

impl AstSourceExtent {
    fn update_location(&mut self, vm: &VirtualMachine, obj: &PyObject) -> PyResult<()> {
        if let Some(lineno) = get_opt_int_field(vm, obj, "lineno")?
            && lineno > 0
        {
            self.max_line = self.max_line.max(lineno as usize);
        }
        if let Some(end_lineno) = get_opt_int_field(vm, obj, "end_lineno")?
            && end_lineno > 0
        {
            self.max_line = self.max_line.max(end_lineno as usize);
        }
        if let Some(col_offset) = get_opt_int_field(vm, obj, "col_offset")?
            && col_offset > 0
        {
            self.max_col = self.max_col.max(col_offset as usize);
        }
        if let Some(end_col_offset) = get_opt_int_field(vm, obj, "end_col_offset")?
            && end_col_offset > 0
        {
            self.max_col = self.max_col.max(end_col_offset as usize);
        }
        Ok(())
    }
}

fn scan_ast_source_extent(
    vm: &VirtualMachine,
    object: &PyObjectRef,
    extent: &mut AstSourceExtent,
) -> PyResult<()> {
    if is_ast_instance(vm, object)? {
        extent.update_location(vm, object)?;
        if let Some(fields) = object.class().get_attr(vm.ctx.intern_str("_fields")) {
            let fields = fields.sequence_unchecked();
            let len = fields.length(vm)?;
            for i in 0..len {
                let field = fields.get_item(i as isize, vm)?;
                if let Some(value) = get_attribute_from_field(vm, object, field)? {
                    vm.with_recursion(" while scanning AST node", || {
                        scan_ast_source_extent(vm, &value, extent)
                    })?;
                }
            }
        }
    } else if let Some(list) = object.downcast_ref::<PyList>() {
        let items = list.borrow_vec().to_vec();
        for item in items {
            vm.with_recursion(" while scanning AST node", || {
                scan_ast_source_extent(vm, &item, extent)
            })?;
        }
    } else if let Some(tuple) = object.downcast_ref::<PyTuple>() {
        for item in tuple.as_slice() {
            vm.with_recursion(" while scanning AST node", || {
                scan_ast_source_extent(vm, item, extent)
            })?;
        }
    }
    Ok(())
}

fn copy_ast_passthrough_fields(
    vm: &VirtualMachine,
    source: &PyObjectRef,
    target: &PyObjectRef,
) -> PyResult<()> {
    if !is_ast_instance(vm, source)?
        || !is_ast_instance(vm, target)?
        || !source.is_instance(target.class().as_object(), vm)?
    {
        return Ok(());
    }

    let fields: &[&str] =
        if is_node_instance(vm, target, pyast::NodeStmtFunctionDef::static_type())?
            || is_node_instance(vm, target, pyast::NodeStmtAsyncFunctionDef::static_type())?
            || is_node_instance(vm, target, pyast::NodeStmtAssign::static_type())?
            || is_node_instance(vm, target, pyast::NodeStmtFor::static_type())?
            || is_node_instance(vm, target, pyast::NodeStmtAsyncFor::static_type())?
            || is_node_instance(vm, target, pyast::NodeStmtWith::static_type())?
            || is_node_instance(vm, target, pyast::NodeStmtAsyncWith::static_type())?
            || is_node_instance(vm, target, pyast::NodeArg::static_type())?
        {
            &["type_comment"]
        } else if is_node_instance(vm, target, pyast::NodeComprehension::static_type())? {
            &["is_async"]
        } else if is_node_instance(vm, target, pyast::NodeExprConstant::static_type())? {
            &["kind"]
        } else if is_node_instance(vm, target, pyast::NodeExprInterpolation::static_type())? {
            &["str"]
        } else {
            &[]
        };

    for field in fields {
        if let Some(value) = vm.get_attribute_opt(source.clone(), *field)? {
            target.set_attr(*field, value, vm)?;
        }
    }

    let Some(source_fields) = source.class().get_attr(vm.ctx.intern_str("_fields")) else {
        return Ok(());
    };
    let Some(target_fields) = target.class().get_attr(vm.ctx.intern_str("_fields")) else {
        return Ok(());
    };
    let source_fields = source_fields.sequence_unchecked();
    let target_fields = target_fields.sequence_unchecked();
    let len = source_fields.length(vm)?;
    if len != target_fields.length(vm)? {
        return Ok(());
    }

    for i in 0..len {
        let source_field = source_fields.get_item(i as isize, vm)?;
        let target_field = target_fields.get_item(i as isize, vm)?;
        if !vm.bool_eq(&source_field, &target_field)? {
            return Ok(());
        }
        let Some(source_value) = get_attribute_from_field(vm, source, source_field)? else {
            continue;
        };
        let Some(target_value) = get_attribute_from_field(vm, target, target_field)? else {
            continue;
        };
        copy_ast_passthrough_children(vm, &source_value, &target_value)?;
    }

    Ok(())
}

fn get_ast_location_field(
    vm: &VirtualMachine,
    object: &PyObjectRef,
    field: &'static str,
) -> PyResult<Option<PyObjectRef>> {
    Ok(vm
        .get_attribute_opt(object.clone(), field)?
        .filter(|value| !vm.is_none(value)))
}

fn ast_start_location_matches(
    vm: &VirtualMachine,
    source: &PyObjectRef,
    target: &PyObjectRef,
) -> PyResult<bool> {
    for field in ["lineno", "col_offset"] {
        let Some(source_value) = get_ast_location_field(vm, source, field)? else {
            return Ok(false);
        };
        let Some(target_value) = get_ast_location_field(vm, target, field)? else {
            return Ok(false);
        };
        if !vm.bool_eq(&source_value, &target_value)? {
            return Ok(false);
        }
    }

    for field in ["end_lineno", "end_col_offset"] {
        let Some(source_value) = get_ast_location_field(vm, source, field)? else {
            continue;
        };
        let Some(target_value) = get_ast_location_field(vm, target, field)? else {
            continue;
        };
        if !vm.bool_eq(&source_value, &target_value)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn ast_passthrough_location_candidate_matches(
    vm: &VirtualMachine,
    source: &PyObjectRef,
    target: &PyObjectRef,
) -> PyResult<bool> {
    Ok(is_ast_instance(vm, source)?
        && is_ast_instance(vm, target)?
        && source.is_instance(target.class().as_object(), vm)?
        && ast_start_location_matches(vm, source, target)?)
}

fn copy_ast_passthrough_list_items_by_location(
    vm: &VirtualMachine,
    source_items: &[PyObjectRef],
    target_items: &[PyObjectRef],
) -> PyResult<()> {
    let mut used_source_items = vec![false; source_items.len()];
    for target_item in target_items {
        for (index, source_item) in source_items.iter().enumerate() {
            if used_source_items[index] {
                continue;
            }
            if ast_passthrough_location_candidate_matches(vm, source_item, target_item)? {
                used_source_items[index] = true;
                copy_ast_passthrough_fields(vm, source_item, target_item)?;
                break;
            }
        }
    }
    Ok(())
}

fn copy_ast_passthrough_children(
    vm: &VirtualMachine,
    source: &PyObjectRef,
    target: &PyObjectRef,
) -> PyResult<()> {
    if is_ast_instance(vm, source)? && is_ast_instance(vm, target)? {
        return copy_ast_passthrough_fields(vm, source, target);
    }

    if let (Some(source_list), Some(target_list)) = (
        source.downcast_ref::<PyList>(),
        target.downcast_ref::<PyList>(),
    ) {
        let source_items = source_list.borrow_vec().to_vec();
        let target_items = target_list.borrow_vec().to_vec();
        if source_items.len() == target_items.len() {
            for (source_item, target_item) in source_items.iter().zip(target_items.iter()) {
                copy_ast_passthrough_children(vm, source_item, target_item)?;
            }
        } else {
            copy_ast_passthrough_list_items_by_location(vm, &source_items, &target_items)?;
        }
    } else if let (Some(source_tuple), Some(target_tuple)) = (
        source.downcast_ref::<PyTuple>(),
        target.downcast_ref::<PyTuple>(),
    ) && source_tuple.as_slice().len() == target_tuple.as_slice().len()
    {
        for (source_item, target_item) in source_tuple
            .as_slice()
            .iter()
            .zip(target_tuple.as_slice().iter())
        {
            copy_ast_passthrough_children(vm, source_item, target_item)?;
        }
    }

    Ok(())
}

fn synthetic_source_from_ast_object(vm: &VirtualMachine, object: &PyObjectRef) -> PyResult<String> {
    let mut extent = AstSourceExtent::default();
    scan_ast_source_extent(vm, object, &mut extent)?;
    if extent.max_line == 0 {
        return Ok(String::new());
    }

    let line_len = extent.max_col.saturating_add(1);
    let line_width = line_len
        .checked_add(1)
        .ok_or_else(|| vm.new_memory_error("source location is too large"))?;
    let capacity = line_width
        .checked_mul(extent.max_line)
        .ok_or_else(|| vm.new_memory_error("source location is too large"))?;
    let mut source = String::new();
    source
        .try_reserve(capacity)
        .map_err(|_| vm.new_memory_error("source location is too large"))?;

    for _ in 0..extent.max_line {
        source.extend(core::iter::repeat_n(' ', line_len));
        source.push('\n');
    }
    Ok(source)
}

fn range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    name: &str,
) -> PyResult<TextRange> {
    range_from_object_impl(vm, source_file, object, name, false)
}

fn type_param_range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<TextRange> {
    range_from_object_impl(vm, source_file, object, "type_param", true)
}

fn expr_range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<TextRange> {
    range_from_object_impl(vm, source_file, object, "expr", false)
}

fn stmt_range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<TextRange> {
    range_from_object_impl(vm, source_file, object, "stmt", false)
}

fn pattern_range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<TextRange> {
    range_from_object_impl(vm, source_file, object, "pattern", true)
}

fn excepthandler_range_from_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<TextRange> {
    range_from_object_impl(vm, source_file, object, "excepthandler", false)
}

fn excepthandler_range_from_object_unvalidated(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<TextRange> {
    let start_row = get_int_field(vm, &object, "lineno", "excepthandler")?;
    let start_column = get_int_field(vm, &object, "col_offset", "excepthandler")?;
    let end_row = get_opt_int_field(vm, &object, "end_lineno")?.unwrap_or(start_row);
    let end_column = get_opt_int_field(vm, &object, "end_col_offset")?.unwrap_or(start_column);

    let location = PySourceRange {
        start: PySourceLocation {
            row: Row(if start_row > 0 {
                OneIndexed::new(start_row as usize).unwrap_or(OneIndexed::MIN)
            } else {
                OneIndexed::MIN
            }),
            column: Column(TextSize::new(start_column.max(0) as u32)),
        },
        end: PySourceLocation {
            row: Row(if end_row > 0 {
                OneIndexed::new(end_row as usize).unwrap_or(OneIndexed::MIN)
            } else {
                OneIndexed::MIN
            }),
            column: Column(TextSize::new(end_column.max(0) as u32)),
        },
    };

    Ok(source_range_to_text_range_unvalidated(
        source_file,
        location,
    ))
}

fn range_from_object_impl(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    name: &str,
    end_required: bool,
) -> PyResult<TextRange> {
    let start_row = get_int_field(vm, &object, "lineno", name)?;
    let start_column = get_int_field(vm, &object, "col_offset", name)?;
    let end_row = if end_required {
        get_int_field(vm, &object, "end_lineno", name)?
    } else {
        get_opt_int_field(vm, &object, "end_lineno")?.unwrap_or(start_row)
    };
    let end_column = if end_required {
        get_int_field(vm, &object, "end_col_offset", name)?
    } else {
        get_opt_int_field(vm, &object, "end_col_offset")?.unwrap_or(start_column)
    };

    // lineno=0 or negative values as a special case (no location info).
    // Use default values (line 1, col 0) when lineno <= 0.
    let start_row_val = start_row;
    let end_row_val = end_row;
    let start_col_val = start_column;
    let end_col_val = end_column;

    if start_row_val > end_row_val {
        return Err(vm.new_value_error(format!(
            "AST node line range ({start_row_val}, {end_row_val}) is not valid"
        )));
    }
    if (start_row_val < 0 && end_row_val != start_row_val)
        || (start_col_val < 0 && end_col_val != start_col_val)
    {
        return Err(vm.new_value_error(format!(
            "AST node column range ({start_col_val}, {end_col_val}) for line range ({start_row_val}, {end_row_val}) is not valid"
        )));
    }
    if start_row_val == end_row_val && start_col_val > end_col_val {
        return Err(vm.new_value_error(format!(
            "line {start_row_val}, column {start_col_val}-{end_col_val} is not a valid range"
        )));
    }

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

fn source_range_to_text_range_unvalidated(
    source_file: &SourceFile,
    location: PySourceRange,
) -> TextRange {
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

    if start <= end {
        TextRange::new(start, end)
    } else {
        TextRange::empty(start)
    }
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

/// Return the expected Python AST root type class for a compile() mode string.
///
/// builtin compile() accepts func_type only with PyCF_ONLY_AST.
/// Source-string func_type parsing is handled separately, but Python AST
/// FunctionType still uses the mode check before obj-to-AST conversion.
pub(crate) fn mode_type_and_name(mode: &str) -> Option<(PyRef<PyType>, &'static str)> {
    match mode {
        "exec" => Some((pyast::NodeModModule::make_static_type(), "Module")),
        "eval" => Some((pyast::NodeModExpression::make_static_type(), "Expression")),
        "single" => Some((pyast::NodeModInteractive::make_static_type(), "Interactive")),
        "func_type" => Some((
            pyast::NodeModFunctionType::make_static_type(),
            "FunctionType",
        )),
        _ => None,
    }
}

struct TypeCommentLine<'a> {
    text: &'a str,
    comment_start: Option<usize>,
}

struct TypeCommentSource<'a> {
    lines: Vec<TypeCommentLine<'a>>,
}

impl<'a> TypeCommentSource<'a> {
    fn new(source: &'a str, tokens: &ast::token::Tokens) -> Self {
        let mut comment_offsets = Vec::new();
        for token in tokens {
            if matches!(token.kind(), ast::token::TokenKind::Comment) {
                comment_offsets.push(token.start().to_usize());
            }
        }

        let mut comment_offsets = comment_offsets.into_iter().peekable();
        let mut line_start = 0usize;
        let mut lines = Vec::new();
        for line in source.split_inclusive('\n') {
            let line_end = line_start + line.len();
            let comment_start = comment_offsets.next_if(|offset| *offset < line_end);
            lines.push(TypeCommentLine {
                text: line,
                comment_start: comment_start.map(|offset| offset - line_start),
            });
            line_start = line_end;
        }

        Self { lines }
    }
}

fn type_comment_position(line: &TypeCommentLine<'_>) -> Option<usize> {
    let comment = line.comment_start?;
    line.text[comment + 1..]
        .trim_start()
        .starts_with("type:")
        .then_some(comment)
}

fn type_comment_text<'a>(line: &'a TypeCommentLine<'a>) -> Option<&'a str> {
    let comment = line.comment_start?;
    let text = line.text.trim_end_matches(['\n', '\r']);
    let mut rest = text[comment + 1..].trim_start_matches([' ', '\t']);
    rest = rest.strip_prefix("type:")?;
    Some(rest.trim_start_matches([' ', '\t']))
}

fn type_ignore_tag(comment: &str) -> Option<&str> {
    let rest = comment.strip_prefix("ignore")?;
    if let Some(next) = rest.as_bytes().first()
        && (next.is_ascii_alphanumeric() || !next.is_ascii())
    {
        return None;
    }
    Some(rest)
}

fn regular_type_comment_text<'a>(line: &'a TypeCommentLine<'a>) -> Option<&'a str> {
    let comment = type_comment_text(line)?;
    type_ignore_tag(comment).is_none().then_some(comment)
}

fn type_comment_parse_error(
    source_file: &SourceFile,
    message: &str,
    start: usize,
    end: usize,
) -> CompileError {
    let range = TextRange::new(TextSize::new(start as u32), TextSize::new(end as u32));
    let source_range = text_range_to_source_range(source_file, range);
    ParseError {
        error: parser::ParseErrorType::OtherError(message.to_owned()),
        raw_location: range,
        location: source_range.start.to_source_location(),
        end_location: source_range.end.to_source_location(),
        source_path: "<unknown>".to_string(),
        is_unclosed_bracket: false,
    }
    .into()
}

#[cfg(feature = "codegen")]
fn future_feature_compile_error(
    source_file: &SourceFile,
    error: codegen::preprocess::FutureFeatureError,
) -> CompileError {
    let location = source_file
        .to_source_code()
        .source_location(error.range.start(), PositionEncoding::Utf8);
    let error = match error.kind {
        codegen::preprocess::FutureFeatureErrorKind::InvalidFeature(feature) => {
            codegen::error::CodegenErrorType::InvalidFutureFeature(feature)
        }
        codegen::preprocess::FutureFeatureErrorKind::InvalidBraces => {
            codegen::error::CodegenErrorType::InvalidFutureBraces
        }
    };
    codegen::error::CodegenError {
        location: Some(location),
        error,
        source_path: source_file.name().to_owned(),
    }
    .into()
}

fn trimmed_line_end(line: &str) -> usize {
    line.trim_end_matches(['\n', '\r']).len()
}

fn line_end_error(
    source_file: &SourceFile,
    message: &str,
    line_start: usize,
    line: &str,
) -> CompileError {
    let start = line_start + trimmed_line_end(line);
    type_comment_parse_error(source_file, message, start, start + 1)
}

fn point_error_end(source: &str, start: usize) -> usize {
    match source.as_bytes().get(start) {
        None => start,
        Some(_) => start + 1,
    }
}

fn line_after_colon_error(
    source_file: &SourceFile,
    message: &str,
    line_start: usize,
    line: &str,
) -> Option<CompileError> {
    let code = &line[..trimmed_line_end(line)];
    let colon = code.rfind(':')?;
    (!code[colon + 1..].trim().is_empty())
        .then(|| line_end_error(source_file, message, line_start, line))
}

fn find_line_containing_offset(source: &str, offset: usize) -> Option<(usize, &str)> {
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if offset < line_end {
            return Some((line_start, line));
        }
        line_start = line_end;
    }
    (offset == source.len()).then_some((line_start, ""))
}

fn find_next_nonempty_line_end(source: &str, offset: usize) -> Option<usize> {
    let (mut line_start, line) = find_line_containing_offset(source, offset)?;
    line_start += line.len();
    for line in source[line_start..].split_inclusive('\n') {
        if !line.trim().is_empty() {
            return Some(line_start + trimmed_line_end(line));
        }
        line_start += line.len();
    }
    None
}

fn find_numeric_literal_containing_underscore(code: &str) -> Option<(usize, usize)> {
    let bytes = code.as_bytes();
    for idx in 1..bytes.len().saturating_sub(1) {
        if bytes[idx] == b'_' && bytes[idx - 1].is_ascii_digit() && bytes[idx + 1].is_ascii_digit()
        {
            let mut start = idx - 1;
            while start > 0
                && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_')
            {
                start -= 1;
            }
            let mut end = idx + 2;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            return Some((start, end));
        }
    }
    None
}

fn bracket_delta(code: &str) -> i32 {
    code.chars().fold(0, |depth, ch| match ch {
        '(' | '[' | '{' => depth + 1,
        ')' | ']' | '}' => depth - 1,
        _ => depth,
    })
}

fn def_header_complete(code: &str, depth: i32) -> bool {
    depth <= 0 && code.trim_end().ends_with(':')
}

fn is_assignment_stmt_line(code: &str) -> bool {
    let bytes = code.as_bytes();
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte != b'=' {
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|idx| bytes.get(idx)).copied();
        let next = bytes.get(idx + 1).copied();
        if matches!(
            prev,
            Some(
                b'=' | b'!'
                    | b'<'
                    | b'>'
                    | b':'
                    | b'+'
                    | b'-'
                    | b'*'
                    | b'/'
                    | b'%'
                    | b'&'
                    | b'|'
                    | b'^'
            )
        ) || matches!(next, Some(b'='))
        {
            continue;
        }
        return true;
    }
    false
}

fn line_allows_stmt_type_comment(code: &str) -> bool {
    let stripped = code.trim_start();
    (stripped.starts_with("for ") || stripped.starts_with("async for ")) && stripped.ends_with(':')
        || (stripped.starts_with("with ") || stripped.starts_with("async with "))
            && stripped.ends_with(':')
        || is_assignment_stmt_line(code)
}

fn invalid_type_comment_syntax_error(
    source_file: &SourceFile,
    type_comment_source: &TypeCommentSource<'_>,
) -> Option<CompileError> {
    let mut line_start = 0usize;
    let mut in_def_header = false;
    let mut def_depth = 0i32;
    let mut pending_func_type_comment = false;
    let mut previous_def_had_type_comment = false;
    for line in &type_comment_source.lines {
        let line_end = line_start + line.text.len();
        let stripped = line.text.trim_start();
        let code_end = type_comment_position(line).unwrap_or(line.text.len());
        let code = line.text[..code_end].trim();
        let has_regular_type_comment = regular_type_comment_text(line).is_some();

        if let Some(comment) = type_comment_position(line) {
            if code == "*" || code == "*," || code.ends_with("*,") {
                return Some(type_comment_parse_error(
                    source_file,
                    "bare * has associated type comment",
                    line_start + comment,
                    line_start + line.text.len(),
                ));
            }
            if previous_def_had_type_comment && code.is_empty() {
                return Some(type_comment_parse_error(
                    source_file,
                    "Cannot have two type comments on def",
                    line_start + comment,
                    line_start + line.text.len(),
                ));
            }
            let allowed = !has_regular_type_comment
                || in_def_header
                || line_allows_stmt_type_comment(code)
                || stripped.starts_with("def ")
                || stripped.starts_with("async def ")
                || (pending_func_type_comment && code.is_empty());
            if !allowed {
                return Some(type_comment_parse_error(
                    source_file,
                    "invalid syntax",
                    line_start + comment,
                    line_start + line.text.len(),
                ));
            }
        }

        let starts_def = stripped.starts_with("def ") || stripped.starts_with("async def ");
        if starts_def && !in_def_header {
            def_depth = bracket_delta(code);
            let complete = def_header_complete(code, def_depth);
            in_def_header = !complete;
            previous_def_had_type_comment = complete && has_regular_type_comment;
            pending_func_type_comment = complete && !has_regular_type_comment;
        } else if in_def_header {
            def_depth += bracket_delta(code);
            let complete = def_header_complete(code, def_depth);
            if complete {
                in_def_header = false;
                previous_def_had_type_comment = has_regular_type_comment;
                pending_func_type_comment = !has_regular_type_comment;
            }
        } else if (pending_func_type_comment && code.is_empty() && has_regular_type_comment)
            || (!stripped.trim().is_empty() && !starts_def && !code.is_empty())
        {
            pending_func_type_comment = false;
            previous_def_had_type_comment = false;
        }

        line_start = line_end;
    }
    None
}

fn feature_version_syntax_error(
    source: &str,
    source_file: &SourceFile,
    target_version: ast::PythonVersion,
) -> Option<CompileError> {
    let mut line_start = 0usize;
    let mut async_def_error = None;
    let mut pending_async_def = false;
    let mut pending_block_error = None;
    for line in source.split_inclusive('\n') {
        let code_end = line.find('#').unwrap_or(line.len());
        let code = &line[..code_end];
        let stripped = code.trim_start();
        if pending_async_def && !stripped.trim().is_empty() {
            if async_def_error.is_none() {
                async_def_error = Some(line_end_error(
                    source_file,
                    "Async functions are only supported in Python 3.5 and greater",
                    line_start,
                    line,
                ));
            }
            pending_async_def = false;
        }
        if let Some(message) = pending_block_error.take() {
            if !stripped.trim().is_empty() {
                return Some(line_end_error(source_file, message, line_start, line));
            }
            pending_block_error = Some(message);
        }

        if target_version.minor < 5 {
            if stripped.starts_with("async def ") && async_def_error.is_none() {
                let message = "Async functions are only supported in Python 3.5 and greater";
                if let Some(error) = line_after_colon_error(source_file, message, line_start, line)
                {
                    async_def_error = Some(error);
                } else {
                    pending_async_def = true;
                }
            }
            if stripped.starts_with("async for ") {
                let message = "Async for loops are only supported in Python 3.5 and greater";
                if let Some(error) = line_after_colon_error(source_file, message, line_start, line)
                {
                    return Some(error);
                }
                pending_block_error = Some(message);
            }
            if stripped.starts_with("async with ") {
                let message = "Async with statements are only supported in Python 3.5 and greater";
                if let Some(error) = line_after_colon_error(source_file, message, line_start, line)
                {
                    return Some(error);
                }
                pending_block_error = Some(message);
            }
            if stripped.starts_with("await ") {
                return Some(line_end_error(
                    source_file,
                    "Await expressions are only supported in Python 3.5 and greater",
                    line_start,
                    line,
                ));
            }
            if let Some(pos) = code.find('@')
                && !stripped.starts_with('@')
            {
                let is_augassign = code.as_bytes().get(pos + 1) == Some(&b'=');
                let (start, end) = if is_augassign {
                    (line_start + pos, line_start + pos + 2)
                } else {
                    let start = line_start + trimmed_line_end(line);
                    (start, start + 1)
                };
                return Some(type_comment_parse_error(
                    source_file,
                    "The '@' operator is only supported in Python 3.5 and greater",
                    start,
                    end,
                ));
            }
        }

        if target_version.minor < 6 {
            if !stripped.starts_with("async for ") && code.contains(" async for ") {
                let start = line_start + trimmed_line_end(line).saturating_sub(1);
                return Some(type_comment_parse_error(
                    source_file,
                    "Async comprehensions are only supported in Python 3.6 and greater",
                    start,
                    point_error_end(source_file.source_text(), start),
                ));
            }
            if let Some((start, end)) = find_numeric_literal_containing_underscore(code) {
                return Some(type_comment_parse_error(
                    source_file,
                    "Underscores in numeric literals are only supported in Python 3.6 and greater",
                    line_start + start,
                    line_start + end,
                ));
            }
        }

        line_start += line.len();
    }
    async_def_error
}

fn ann_assign_feature_error(stmts: &[ast::Stmt], source_file: &SourceFile) -> Option<CompileError> {
    for stmt in stmts {
        match stmt {
            ast::Stmt::AnnAssign(ann) => {
                let start = ann.range().end().to_usize();
                return Some(type_comment_parse_error(
                    source_file,
                    "Variable annotation syntax is only supported in Python 3.6 and greater",
                    start,
                    point_error_end(source_file.source_text(), start),
                ));
            }
            ast::Stmt::FunctionDef(def) => {
                if let Some(error) = ann_assign_feature_error(&def.body, source_file) {
                    return Some(error);
                }
            }
            ast::Stmt::ClassDef(class_def) => {
                if let Some(error) = ann_assign_feature_error(&class_def.body, source_file) {
                    return Some(error);
                }
            }
            ast::Stmt::For(for_stmt) => {
                if let Some(error) = ann_assign_feature_error(&for_stmt.body, source_file)
                    .or_else(|| ann_assign_feature_error(&for_stmt.orelse, source_file))
                {
                    return Some(error);
                }
            }
            ast::Stmt::While(while_stmt) => {
                if let Some(error) = ann_assign_feature_error(&while_stmt.body, source_file)
                    .or_else(|| ann_assign_feature_error(&while_stmt.orelse, source_file))
                {
                    return Some(error);
                }
            }
            ast::Stmt::If(if_stmt) => {
                if let Some(error) = ann_assign_feature_error(&if_stmt.body, source_file) {
                    return Some(error);
                }
                for clause in &if_stmt.elif_else_clauses {
                    if let Some(error) = ann_assign_feature_error(&clause.body, source_file) {
                        return Some(error);
                    }
                }
            }
            ast::Stmt::With(with_stmt) => {
                if let Some(error) = ann_assign_feature_error(&with_stmt.body, source_file) {
                    return Some(error);
                }
            }
            ast::Stmt::Match(match_stmt) => {
                for case in &match_stmt.cases {
                    if let Some(error) = ann_assign_feature_error(&case.body, source_file) {
                        return Some(error);
                    }
                }
            }
            ast::Stmt::Try(try_stmt) => {
                if let Some(error) = ann_assign_feature_error(&try_stmt.body, source_file)
                    .or_else(|| ann_assign_feature_error(&try_stmt.orelse, source_file))
                    .or_else(|| ann_assign_feature_error(&try_stmt.finalbody, source_file))
                {
                    return Some(error);
                }
                for handler in &try_stmt.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    if let Some(error) = ann_assign_feature_error(&handler.body, source_file) {
                        return Some(error);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn feature_version_ast_syntax_error(
    top: &ast::Mod,
    source_file: &SourceFile,
    target_version: ast::PythonVersion,
) -> Option<CompileError> {
    if target_version.minor >= 6 {
        return None;
    }
    match top {
        ast::Mod::Module(module) => ann_assign_feature_error(&module.body, source_file),
        ast::Mod::Expression(_) => None,
    }
}

fn cpython_unsupported_syntax_message(
    error: &parser::UnsupportedSyntaxError,
) -> Option<&'static str> {
    match error.kind {
        parser::UnsupportedSyntaxErrorKind::Match => {
            Some("Pattern matching is only supported in Python 3.10 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::Walrus => {
            Some("Assignment expressions are only supported in Python 3.8 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::ExceptStar => {
            Some("Exception groups are only supported in Python 3.11 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::PositionalOnlyParameter => {
            Some("Positional-only parameters are only supported in Python 3.8 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::TypeParameterList => {
            Some("Type parameter lists are only supported in Python 3.12 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::TypeAliasStatement => {
            Some("Type statement is only supported in Python 3.12 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::TypeParamDefault => {
            Some("Type parameter defaults are only supported in Python 3.13 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::TemplateStrings => {
            Some("t-strings are only supported in Python 3.14 and greater")
        }
        parser::UnsupportedSyntaxErrorKind::UnparenthesizedExceptionTypes => Some(
            "except expressions without parentheses are only supported in Python 3.14 and greater",
        ),
        _ => None,
    }
}

fn cpython_unsupported_syntax_error(
    error: &parser::UnsupportedSyntaxError,
    source: &str,
    source_file: &SourceFile,
) -> Option<CompileError> {
    let message = cpython_unsupported_syntax_message(error)?;
    let start = match error.kind {
        parser::UnsupportedSyntaxErrorKind::Match
        | parser::UnsupportedSyntaxErrorKind::ExceptStar
        | parser::UnsupportedSyntaxErrorKind::UnparenthesizedExceptionTypes => {
            find_next_nonempty_line_end(source, error.range.start().to_usize())
                .unwrap_or_else(|| error.range.end().to_usize())
        }
        parser::UnsupportedSyntaxErrorKind::Walrus
        | parser::UnsupportedSyntaxErrorKind::PositionalOnlyParameter
        | parser::UnsupportedSyntaxErrorKind::TypeParamDefault => error.range.end().to_usize(),
        parser::UnsupportedSyntaxErrorKind::TypeAliasStatement => {
            let (line_start, line) =
                find_line_containing_offset(source, error.range.start().to_usize())?;
            line_start + trimmed_line_end(line)
        }
        parser::UnsupportedSyntaxErrorKind::TypeParameterList => {
            let (line_start, line) =
                find_line_containing_offset(source, error.range.start().to_usize())?;
            let code = &line[..trimmed_line_end(line)];
            line_start
                + code
                    .as_bytes()
                    .iter()
                    .rposition(|byte| *byte == b']')
                    .unwrap_or_else(|| error.range.end().to_usize() - line_start)
        }
        parser::UnsupportedSyntaxErrorKind::TemplateStrings => {
            let (line_start, line) =
                find_line_containing_offset(source, error.range.start().to_usize())?;
            line_start + trimmed_line_end(line).saturating_sub(1)
        }
        _ => error.range.start().to_usize(),
    };
    Some(type_comment_parse_error(
        source_file,
        message,
        start,
        point_error_end(source, start),
    ))
}

fn should_report_unsupported_syntax_error(error: &parser::UnsupportedSyntaxError) -> bool {
    cpython_unsupported_syntax_message(error).is_some()
        || matches!(
            error.kind,
            parser::UnsupportedSyntaxErrorKind::LazyImportStatement
                | parser::UnsupportedSyntaxErrorKind::ParenthesizedKeywordArgumentName
        )
}

fn node_list_field(
    vm: &VirtualMachine,
    object: &PyObjectRef,
    field: &'static str,
) -> Vec<PyObjectRef> {
    vm.get_attribute_opt(object.clone(), field)
        .ok()
        .flatten()
        .and_then(|value| {
            value
                .downcast_ref::<PyList>()
                .map(|list| list.borrow_vec().to_vec())
        })
        .unwrap_or_default()
}

fn node_optional_field(
    vm: &VirtualMachine,
    object: &PyObjectRef,
    field: &'static str,
) -> Option<PyObjectRef> {
    vm.get_attribute_opt(object.clone(), field)
        .ok()
        .flatten()
        .filter(|value| !vm.is_none(value))
}

fn node_lineno(vm: &VirtualMachine, object: &PyObjectRef) -> Option<usize> {
    node_optional_field(vm, object, "lineno")?
        .try_into_value(vm)
        .ok()
}

fn source_line<'a>(
    lines: &'a TypeCommentSource<'a>,
    lineno: usize,
) -> Option<&'a TypeCommentLine<'a>> {
    lineno.checked_sub(1).and_then(|idx| lines.lines.get(idx))
}

fn set_type_comment(vm: &VirtualMachine, object: &PyObjectRef, comment: Option<&str>) {
    let value = comment.map_or_else(|| vm.ctx.none(), |comment| vm.ctx.new_str(comment).into());
    object
        .as_object()
        .dict()
        .unwrap()
        .set_item("type_comment", value, vm)
        .unwrap();
}

fn same_line_type_comment<'a>(
    vm: &VirtualMachine,
    lines: &'a TypeCommentSource<'a>,
    object: &PyObjectRef,
) -> Option<&'a str> {
    let lineno = node_lineno(vm, object)?;
    regular_type_comment_text(source_line(lines, lineno)?)
}

fn function_type_comment<'a>(
    vm: &VirtualMachine,
    lines: &'a TypeCommentSource<'a>,
    object: &PyObjectRef,
) -> Option<&'a str> {
    let lineno = node_lineno(vm, object)?;
    if let Some(comment) = regular_type_comment_text(source_line(lines, lineno)?) {
        return Some(comment);
    }

    let next_line = source_line(lines, lineno + 1)?;
    let comment_pos = type_comment_position(next_line)?;
    next_line.text[..comment_pos]
        .trim()
        .is_empty()
        .then(|| regular_type_comment_text(next_line))
        .flatten()
}

fn apply_type_comments_to_arguments(
    vm: &VirtualMachine,
    lines: &TypeCommentSource<'_>,
    arguments: &PyObjectRef,
) {
    for field in ["posonlyargs", "args", "kwonlyargs"] {
        for arg in node_list_field(vm, arguments, field) {
            set_type_comment(vm, &arg, same_line_type_comment(vm, lines, &arg));
        }
    }
    for field in ["vararg", "kwarg"] {
        if let Some(arg) = node_optional_field(vm, arguments, field) {
            set_type_comment(vm, &arg, same_line_type_comment(vm, lines, &arg));
        }
    }
}

fn apply_type_comments_to_node(
    vm: &VirtualMachine,
    lines: &TypeCommentSource<'_>,
    object: &PyObjectRef,
) {
    let cls = object.class();
    if cls.is(pyast::NodeStmtFunctionDef::static_type())
        || cls.is(pyast::NodeStmtAsyncFunctionDef::static_type())
    {
        set_type_comment(vm, object, function_type_comment(vm, lines, object));
        if let Some(arguments) = node_optional_field(vm, object, "args") {
            apply_type_comments_to_arguments(vm, lines, &arguments);
        }
    } else if cls.is(pyast::NodeStmtAssign::static_type())
        || cls.is(pyast::NodeStmtFor::static_type())
        || cls.is(pyast::NodeStmtAsyncFor::static_type())
        || cls.is(pyast::NodeStmtWith::static_type())
        || cls.is(pyast::NodeStmtAsyncWith::static_type())
    {
        set_type_comment(vm, object, same_line_type_comment(vm, lines, object));
    }

    for field in ["body", "orelse", "finalbody"] {
        for child in node_list_field(vm, object, field) {
            apply_type_comments_to_node(vm, lines, &child);
        }
    }
    for field in ["handlers", "cases"] {
        for child in node_list_field(vm, object, field) {
            apply_type_comments_to_node(vm, lines, &child);
        }
    }
}

fn apply_type_comments_to_module(
    vm: &VirtualMachine,
    lines: &TypeCommentSource<'_>,
    module: &PyObjectRef,
) {
    for statement in node_list_field(vm, module, "body") {
        apply_type_comments_to_node(vm, lines, &statement);
    }
}

#[cfg(feature = "parser")]
fn ipython_escape_command_syntax_error(
    top: &ast::Mod,
    source_file: &SourceFile,
) -> Option<CompileError> {
    use ast::visitor::{Visitor, walk_expr, walk_stmt};

    #[derive(Default)]
    struct IpyEscapeCommandVisitor {
        range: Option<TextRange>,
    }

    impl Visitor<'_> for IpyEscapeCommandVisitor {
        fn visit_stmt(&mut self, stmt: &ast::Stmt) {
            if self.range.is_some() {
                return;
            }
            match stmt {
                ast::Stmt::IpyEscapeCommand(stmt) => {
                    self.range = Some(stmt.range);
                }
                _ => walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &ast::Expr) {
            if self.range.is_some() {
                return;
            }
            match expr {
                ast::Expr::IpyEscapeCommand(expr) => {
                    self.range = Some(expr.range);
                }
                _ => walk_expr(self, expr),
            }
        }
    }

    let mut visitor = IpyEscapeCommandVisitor::default();
    match top {
        ast::Mod::Module(module) => {
            for statement in &module.body {
                visitor.visit_stmt(statement);
                if visitor.range.is_some() {
                    break;
                }
            }
        }
        ast::Mod::Expression(expression) => {
            visitor.visit_expr(&expression.body);
        }
    }
    let range = visitor.range?;
    let source_range = text_range_to_source_range(source_file, range);
    Some(
        ParseError {
            error: parser::ParseErrorType::OtherError("invalid syntax".to_owned()),
            raw_location: range,
            location: source_range.start.to_source_location(),
            end_location: source_range.end.to_source_location(),
            source_path: "<unknown>".to_owned(),
            is_unclosed_bracket: false,
        }
        .into(),
    )
}

/// Create an empty `arguments` AST node (no parameters).
fn empty_arguments_object(vm: &VirtualMachine) -> PyObjectRef {
    let node = NodeAst
        .into_ref_with_type(vm, pyast::NodeArguments::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();
    for list_field in [
        "posonlyargs",
        "args",
        "kwonlyargs",
        "kw_defaults",
        "defaults",
    ] {
        dict.set_item(list_field, vm.ctx.new_list(vec![]).into(), vm)
            .unwrap();
    }
    for none_field in ["vararg", "kwarg"] {
        dict.set_item(none_field, vm.ctx.none(), vm).unwrap();
    }
    node.into()
}

#[cfg(feature = "parser")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse(
    vm: &VirtualMachine,
    source: &str,
    mode: parser::Mode,
    optimize: u8,
    target_version: Option<ast::PythonVersion>,
    type_comments: bool,
    optimized_ast: bool,
    interactive: bool,
    explicit_future_annotations: bool,
    dont_imply_dedent: bool,
) -> Result<PyObjectRef, CompileError> {
    let source_file = SourceFileBuilder::new("".to_owned(), source.to_owned()).finish();
    let mut options = parser::ParseOptions::from(mode);
    let target_version = target_version.unwrap_or(ast::PythonVersion::PY314);
    if let Some(error) = feature_version_syntax_error(source, &source_file, target_version) {
        return Err(error);
    }
    options = options.with_target_version(target_version);
    let parsed = parser::parse_unchecked(source, options);
    let type_comment_source =
        type_comments.then(|| TypeCommentSource::new(source, parsed.tokens()));
    if let Some(lines) = &type_comment_source
        && let Some(error) = invalid_type_comment_syntax_error(&source_file, lines)
    {
        return Err(error);
    }
    if let Err(errors) = parsed.as_result() {
        let parse_error = errors[0].clone();
        let range = text_range_to_source_range(&source_file, parse_error.location);
        return Err(ParseError {
            error: parse_error.error,
            raw_location: parse_error.location,
            location: range.start.to_source_location(),
            end_location: range.end.to_source_location(),
            source_path: "<unknown>".to_string(),
            is_unclosed_bracket: false,
        }
        .into());
    }
    if dont_imply_dedent
        && interactive
        && let Some(error) = rustpython_compiler::dont_imply_dedent_source_error(&source_file)
    {
        return Err(error);
    }

    if let Some(error) = parsed
        .unsupported_syntax_errors()
        .iter()
        .find(|error| should_report_unsupported_syntax_error(error))
    {
        if let Some(error) = cpython_unsupported_syntax_error(error, source, &source_file) {
            return Err(error);
        }
        let range = text_range_to_source_range(&source_file, error.range());
        return Err(ParseError {
            error: parser::ParseErrorType::OtherError(error.to_string()),
            raw_location: error.range(),
            location: range.start.to_source_location(),
            end_location: range.end.to_source_location(),
            source_path: "<unknown>".to_string(),
            is_unclosed_bracket: false,
        }
        .into());
    }

    let mut top = parsed.into_syntax();
    if let Some(error) = ipython_escape_command_syntax_error(&top, &source_file) {
        return Err(error);
    }
    if let Some(error) = feature_version_ast_syntax_error(&top, &source_file, target_version) {
        return Err(error);
    }
    #[cfg(feature = "codegen")]
    {
        let future_features = codegen::preprocess::checked_future_features(&top)
            .map_err(|err| future_feature_compile_error(&source_file, err))?;
        let future_annotations = explicit_future_annotations
            || future_features.contains(crate::bytecode::CodeFlags::FUTURE_ANNOTATIONS);
        if interactive && let ast::Mod::Module(module) = &mut top {
            codegen::preprocess::preprocess_statements(
                &mut module.body,
                optimize,
                future_annotations,
                !optimized_ast,
            );
        } else {
            codegen::preprocess::preprocess_mod(
                &mut top,
                optimize,
                future_annotations,
                !optimized_ast,
            );
        }
    }
    #[cfg(not(feature = "codegen"))]
    {
        if optimized_ast && optimize > 0 {
            fold_match_value_constants(&mut top);
        }
        if optimize >= 2 {
            strip_docstrings(&mut top);
        }
    }
    let top = match top {
        ast::Mod::Module(m) => Mod::Module(ModModule {
            module: m,
            type_ignores: Vec::new(),
        }),
        ast::Mod::Expression(e) => Mod::Expression(e),
    };
    let obj = top.ast_to_object(vm, &source_file);
    if let Some(lines) = &type_comment_source
        && obj.class().is(pyast::NodeModModule::static_type())
    {
        apply_type_comments_to_module(vm, lines, &obj);
        let type_ignores = type_ignores_from_source(vm, lines);
        let dict = obj.as_object().dict().unwrap();
        dict.set_item("type_ignores", vm.ctx.new_list(type_ignores).into(), vm)
            .unwrap();
    }
    Ok(obj)
}

#[cfg(feature = "parser")]
pub(crate) fn wrap_interactive(vm: &VirtualMachine, module_obj: PyObjectRef) -> PyResult {
    if !module_obj.class().is(pyast::NodeModModule::static_type()) {
        return Err(vm.new_type_error("expected Module node"));
    }
    let body = get_node_field(vm, &module_obj, "body", "Module")?;
    let node = NodeAst
        .into_ref_with_type(vm, pyast::NodeModInteractive::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();
    dict.set_item("body", body, vm).unwrap();
    Ok(node.into())
}

#[cfg(feature = "parser")]
pub(crate) fn parse_func_type(
    vm: &VirtualMachine,
    source: &str,
    optimize: u8,
    target_version: Option<ast::PythonVersion>,
) -> Result<PyObjectRef, CompileError> {
    let _ = optimize;
    let source = source.trim();
    let invalid_func_type = || -> CompileError {
        ParseError {
            error: parser::ParseErrorType::OtherError("invalid syntax".to_owned()),
            raw_location: TextRange::default(),
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            source_path: "<unknown>".to_owned(),
            is_unclosed_bracket: false,
        }
        .into()
    };
    let mut depth = 0i32;
    let mut split_at = None;
    let mut chars = source.chars().peekable();
    let mut idx = 0usize;
    while let Some(ch) = chars.next() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '-' if depth == 0 && chars.peek() == Some(&'>') => {
                split_at = Some(idx);
                break;
            }
            _ => {}
        }
        idx += ch.len_utf8();
    }

    let Some(split_at) = split_at else {
        return Err(ParseError {
            error: parser::ParseErrorType::OtherError("invalid func_type".to_owned()),
            raw_location: TextRange::default(),
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            source_path: "<unknown>".to_owned(),
            is_unclosed_bracket: false,
        }
        .into());
    };

    let left = source[..split_at].trim();
    let right = source[split_at + 2..].trim();

    let parse_expr = |expr_src: &str| -> Result<ast::Expr, CompileError> {
        let source_file = SourceFileBuilder::new("".to_owned(), expr_src.to_owned()).finish();
        let options = parser::ParseOptions::from(parser::Mode::Expression)
            .with_target_version(target_version.unwrap_or(ast::PythonVersion::PY314));
        let parsed = parser::parse(expr_src, options).map_err(|parse_error| {
            let range = text_range_to_source_range(&source_file, parse_error.location);
            ParseError {
                error: parse_error.error,
                raw_location: parse_error.location,
                location: range.start.to_source_location(),
                end_location: range.end.to_source_location(),
                source_path: "<unknown>".to_string(),
                is_unclosed_bracket: false,
            }
        })?;
        let ast::Mod::Expression(expression) = parsed.into_syntax() else {
            unreachable!();
        };
        Ok(*expression.body)
    };

    if !left.starts_with('(') || !left.ends_with(')') {
        return Err(invalid_func_type());
    }
    let inner = left[1..left.len() - 1].trim();
    let argtypes = if inner.is_empty() {
        Vec::new()
    } else {
        if inner.ends_with(',') {
            return Err(invalid_func_type());
        }
        let call_source = format!("__rustpython_func_type__({inner})");
        let source_file = SourceFileBuilder::new("".to_owned(), call_source.clone()).finish();
        let options = parser::ParseOptions::from(parser::Mode::Expression)
            .with_target_version(target_version.unwrap_or(ast::PythonVersion::PY314));
        let parsed = parser::parse(&call_source, options).map_err(|parse_error| {
            let range = text_range_to_source_range(&source_file, parse_error.location);
            ParseError {
                error: parse_error.error,
                raw_location: parse_error.location,
                location: range.start.to_source_location(),
                end_location: range.end.to_source_location(),
                source_path: "<unknown>".to_string(),
                is_unclosed_bracket: false,
            }
        })?;
        let ast::Mod::Expression(expression) = parsed.into_syntax() else {
            unreachable!();
        };
        let ast::Expr::Call(call) = *expression.body else {
            return Err(invalid_func_type());
        };
        let mut args = Vec::new();
        let positional_len = call.arguments.args.len();
        let mut seen_star = false;
        for (index, arg) in call.arguments.args.into_iter().enumerate() {
            match arg {
                ast::Expr::Starred(starred) => {
                    if seen_star || index + 1 != positional_len {
                        return Err(invalid_func_type());
                    }
                    seen_star = true;
                    args.push(*starred.value);
                }
                expr => args.push(expr),
            }
        }
        let mut seen_kw_star = false;
        for keyword in call.arguments.keywords {
            if keyword.arg.is_some() || seen_kw_star {
                return Err(invalid_func_type());
            }
            seen_kw_star = true;
            args.push(keyword.value);
        }
        args
    };

    let returns = parse_expr(right)?;

    let func_type = ModFunctionType {
        argtypes: argtypes.into_boxed_slice(),
        returns,
        runtime_argtypes: None,
    };
    let source_file = SourceFileBuilder::new("".to_owned(), source.to_owned()).finish();
    Ok(func_type.ast_to_object(vm, &source_file))
}

fn type_ignores_from_source(
    vm: &VirtualMachine,
    lines: &TypeCommentSource<'_>,
) -> Vec<PyObjectRef> {
    let mut ignores = Vec::new();
    for (idx, line) in lines.lines.iter().enumerate() {
        let Some(comment) = type_comment_text(line) else {
            continue;
        };
        let Some(tag) = type_ignore_tag(comment) else {
            continue;
        };
        let node = NodeAst
            .into_ref_with_type(
                vm,
                pyast::NodeTypeIgnoreTypeIgnore::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let lineno = idx + 1;
        dict.set_item("lineno", vm.ctx.new_int(lineno).into(), vm)
            .unwrap();
        dict.set_item("tag", vm.ctx.new_str(tag).into(), vm)
            .unwrap();
        ignores.push(node.into());
    }
    ignores
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn fold_match_value_constants(top: &mut ast::Mod) {
    match top {
        ast::Mod::Module(module) => fold_stmts(&mut module.body),
        ast::Mod::Expression(_expr) => {}
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn strip_docstrings(top: &mut ast::Mod) {
    match top {
        ast::Mod::Module(module) => strip_docstring_in_body(&mut module.body),
        ast::Mod::Expression(_expr) => {}
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn strip_docstring_in_body(body: &mut ast::Suite) {
    if let Some(range) = take_docstring(body)
        && body.is_empty()
    {
        let start_offset = range.start();
        let end_offset = start_offset + TextSize::from(4);
        let pass_range = TextRange::new(start_offset, end_offset);
        body.push(ast::Stmt::Pass(ast::StmtPass {
            node_index: Default::default(),
            range: pass_range,
        }));
    }
    for stmt in body {
        match stmt {
            ast::Stmt::FunctionDef(def) => strip_docstring_in_body(&mut def.body),
            ast::Stmt::ClassDef(def) => strip_docstring_in_body(&mut def.body),
            _ => {}
        }
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn take_docstring(body: &mut ast::Suite) -> Option<TextRange> {
    let ast::Stmt::Expr(expr_stmt) = body.first()? else {
        return None;
    };
    if matches!(
        expr_stmt.value.as_ref(),
        ast::Expr::StringLiteral(_)
            | ast::Expr::Constant(ast::ExprConstant {
                value: ast::ConstantValue::Str(_),
                ..
            })
    ) {
        let range = expr_stmt.range;
        body.remove(0);
        return Some(range);
    }
    None
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn fold_stmts(stmts: &mut [ast::Stmt]) {
    for stmt in stmts {
        fold_stmt(stmt);
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn fold_stmt(stmt: &mut ast::Stmt) {
    use ast::Stmt;
    match stmt {
        Stmt::FunctionDef(def) => fold_stmts(&mut def.body),
        Stmt::ClassDef(def) => fold_stmts(&mut def.body),
        Stmt::For(stmt) => {
            fold_stmts(&mut stmt.body);
            fold_stmts(&mut stmt.orelse);
        }
        Stmt::While(stmt) => {
            fold_stmts(&mut stmt.body);
            fold_stmts(&mut stmt.orelse);
        }
        Stmt::If(stmt) => {
            fold_stmts(&mut stmt.body);
            for clause in &mut stmt.elif_else_clauses {
                fold_stmts(&mut clause.body);
            }
        }
        Stmt::With(stmt) => {
            fold_stmts(&mut stmt.body);
        }
        Stmt::Try(stmt) => {
            fold_stmts(&mut stmt.body);
            fold_stmts(&mut stmt.orelse);
            fold_stmts(&mut stmt.finalbody);
        }
        Stmt::Match(stmt) => {
            for case in &mut stmt.cases {
                fold_pattern(&mut case.pattern);
                if let Some(expr) = case.guard.as_deref_mut() {
                    fold_expr(expr);
                }
                fold_stmts(&mut case.body);
            }
        }
        _ => {}
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn fold_pattern(pattern: &mut ast::Pattern) {
    use ast::Pattern;
    match pattern {
        Pattern::MatchValue(value) => fold_expr(&mut value.value),
        Pattern::MatchSequence(seq) => {
            for pattern in &mut seq.patterns {
                fold_pattern(pattern);
            }
        }
        Pattern::MatchMapping(mapping) => {
            for key in &mut mapping.keys {
                fold_expr(key);
            }
            for pattern in &mut mapping.patterns {
                fold_pattern(pattern);
            }
        }
        Pattern::MatchClass(class) => {
            for pattern in &mut class.arguments.patterns {
                fold_pattern(pattern);
            }
            for keyword in &mut class.arguments.keywords {
                fold_pattern(&mut keyword.pattern);
            }
        }
        Pattern::MatchAs(match_as) => {
            if let Some(pattern) = match_as.pattern.as_deref_mut() {
                fold_pattern(pattern);
            }
        }
        Pattern::MatchOr(match_or) => {
            for pattern in &mut match_or.patterns {
                fold_pattern(pattern);
            }
        }
        Pattern::MatchSingleton(_) | Pattern::MatchStar(_) => {}
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn fold_expr(expr: &mut ast::Expr) {
    use ast::Expr;
    if let Expr::UnaryOp(unary) = expr {
        fold_expr(&mut unary.operand);
        if matches!(unary.op, ast::UnaryOp::USub)
            && let Expr::NumberLiteral(number_literal) = unary.operand.as_ref()
        {
            let number = match &number_literal.value {
                ast::Number::Int(value) => {
                    if *value == ast::Int::ZERO {
                        Some(ast::Number::Int(ast::Int::ZERO))
                    } else {
                        None
                    }
                }
                ast::Number::Float(value) => Some(ast::Number::Float(-value)),
                ast::Number::Complex { real, imag } => Some(ast::Number::Complex {
                    real: -real,
                    imag: -imag,
                }),
            };
            if let Some(number) = number {
                *expr = Expr::NumberLiteral(ast::ExprNumberLiteral {
                    node_index: unary.node_index.clone(),
                    range: unary.range,
                    value: number,
                });
                return;
            }
        }
    }
    if let Expr::BinOp(binop) = expr {
        fold_expr(&mut binop.left);
        fold_expr(&mut binop.right);

        let Expr::NumberLiteral(left) = binop.left.as_ref() else {
            return;
        };

        let Expr::NumberLiteral(right) = binop.right.as_ref() else {
            return;
        };

        if let Some(number) = fold_number_binop(&left.value, binop.op, &right.value) {
            *expr = Expr::NumberLiteral(ast::ExprNumberLiteral {
                node_index: binop.node_index.clone(),
                range: binop.range,
                value: number,
            });
        }
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn fold_number_binop(
    left: &ast::Number,
    op: ast::Operator,
    right: &ast::Number,
) -> Option<ast::Number> {
    let (left_real, left_imag, left_is_complex) = number_to_complex(left)?;
    let (right_real, right_imag, right_is_complex) = number_to_complex(right)?;

    if !(left_is_complex || right_is_complex) {
        return None;
    }

    match op {
        ast::Operator::Add => Some(ast::Number::Complex {
            real: left_real + right_real,
            imag: left_imag + right_imag,
        }),
        ast::Operator::Sub => Some(ast::Number::Complex {
            real: left_real - right_real,
            imag: left_imag - right_imag,
        }),
        _ => None,
    }
}

#[cfg(all(feature = "parser", not(feature = "codegen")))]
fn number_to_complex(number: &ast::Number) -> Option<(f64, f64, bool)> {
    match number {
        ast::Number::Complex { real, imag } => Some((*real, *imag, true)),
        ast::Number::Float(value) => Some((*value, 0.0, false)),
        ast::Number::Int(value) => value.as_i64().map(|value| (value as f64, 0.0, false)),
    }
}

#[cfg(feature = "codegen")]
pub(crate) fn preprocess_ast_object(
    vm: &VirtualMachine,
    object: PyObjectRef,
    filename: &str,
    optimize: u8,
    optimized_ast: bool,
    explicit_future_annotations: bool,
) -> PyResult<PyObjectRef> {
    let original_object = object.clone();
    let text = synthetic_source_from_ast_object(vm, &object)?;
    let source_file = SourceFileBuilder::new(filename.to_owned(), text).finish();
    let ast = Node::ast_from_object(vm, &source_file, object)?;
    validate::validate_mod(vm, &ast)?;
    let syntax_check_only = !optimized_ast;

    let ast = match ast {
        Mod::Module(mut module) => {
            let mut ast = ast::Mod::Module(module.module);
            let future_features =
                codegen::preprocess::checked_future_features(&ast).map_err(|err| {
                    vm.new_syntax_error(&future_feature_compile_error(&source_file, err), None)
                })?;
            let future_annotations = explicit_future_annotations
                || future_features.contains(crate::bytecode::CodeFlags::FUTURE_ANNOTATIONS);
            codegen::preprocess::preprocess_mod(
                &mut ast,
                optimize,
                future_annotations,
                syntax_check_only,
            );
            let ast::Mod::Module(processed_module) = ast else {
                unreachable!();
            };
            module.module = processed_module;
            Mod::Module(module)
        }
        Mod::Interactive(mut interactive) => {
            let future_features = codegen::preprocess::checked_future_features_in_body(
                &interactive.body,
            )
            .map_err(|err| {
                vm.new_syntax_error(&future_feature_compile_error(&source_file, err), None)
            })?;
            let future_annotations = explicit_future_annotations
                || future_features.contains(crate::bytecode::CodeFlags::FUTURE_ANNOTATIONS);
            codegen::preprocess::preprocess_statements(
                &mut interactive.body,
                optimize,
                future_annotations,
                syntax_check_only,
            );
            Mod::Interactive(interactive)
        }
        Mod::Expression(expression) => {
            let mut ast = ast::Mod::Expression(expression);
            codegen::preprocess::preprocess_mod(
                &mut ast,
                optimize,
                explicit_future_annotations,
                syntax_check_only,
            );
            let ast::Mod::Expression(expression) = ast else {
                unreachable!();
            };
            Mod::Expression(expression)
        }
        Mod::FunctionType(function_type) => Mod::FunctionType(function_type),
    };
    let result = ast.ast_to_object(vm, &source_file);
    copy_ast_passthrough_fields(vm, &original_object, &result)?;
    Ok(result)
}

#[cfg(feature = "codegen")]
pub(crate) fn compile(
    vm: &VirtualMachine,
    object: PyObjectRef,
    filename: &str,
    mode: crate::compiler::Mode,
    mut opts: codegen::CompileOpts,
) -> PyResult {
    let text = synthetic_source_from_ast_object(vm, &object)?;
    let source_file = SourceFileBuilder::new(filename.to_owned(), text.clone()).finish();
    let ast = Node::ast_from_object(vm, &source_file, object)?;
    validate::validate_mod(vm, &ast)?;
    let ast = match ast {
        Mod::Module(m) => ast::Mod::Module(m.module),
        Mod::Interactive(ModInteractive { range, body, .. }) => ast::Mod::Module(ast::ModModule {
            node_index: Default::default(),
            range,
            body,
            runtime_body: None,
        }),
        Mod::Expression(e) => ast::Mod::Expression(e),
        Mod::FunctionType(_) => {
            return Err(vm.new_runtime_error("this compiler does not handle FunctionTypes"));
        }
    };
    opts.future_features |= codegen::preprocess::future_features(&ast);
    let source = text.clone();
    let source_file = SourceFileBuilder::new(filename, text).finish();
    #[cfg(feature = "parser")]
    let code = {
        let source_path = filename.to_owned();
        // A warning the filter escalates to an exception is stashed here so a
        // non-SyntaxWarning category propagates unchanged, matching
        // PyErr_ExceptionMatches(SyntaxWarning) in compiler_warn.
        let escalated: core::cell::Cell<Option<crate::builtins::PyBaseExceptionRef>> =
            core::cell::Cell::new(None);
        let mut syntax_warning_handler = |location: SourceLocation, message: String| {
            let fname = vm.ctx.new_str(source_path.as_str());
            let message = vm.ctx.new_str(message);
            crate::warn::warn_explicit(
                Some(vm.ctx.exceptions.syntax_warning.to_owned()),
                message.into(),
                fname,
                location.line.get(),
                None,
                vm.ctx.none(),
                None,
                None,
                vm,
            )
            .map_err(|exception| {
                let message = exception.as_object().str(vm).map_or_else(
                    |_| "compiler warning raised as an exception".to_owned(),
                    |message| message.as_wtf8().to_string(),
                );
                let marker = codegen::error::CodegenError {
                    location: Some(location),
                    error: codegen::error::CodegenErrorType::SyntaxError(message),
                    source_path: source_path.clone(),
                };
                escalated.set(Some(exception));
                marker
            })
        };
        let result = codegen::compile::compile_top_with_syntax_warning_handler(
            ast,
            source_file,
            mode,
            opts,
            Some(&mut syntax_warning_handler),
        );
        match escalated.take() {
            Some(exception) if !exception.fast_isinstance(vm.ctx.exceptions.syntax_warning) => {
                return Err(exception);
            }
            _ => result,
        }
    };
    #[cfg(not(feature = "parser"))]
    let code = codegen::compile::compile_top(ast, source_file, mode, opts);
    let code = code.map_err(|err| vm.new_syntax_error(&err.into(), Some(source.as_str())))?;
    Ok(crate::builtins::PyCode::new_ref_from_bytecode(vm, code).into())
}

#[cfg(not(feature = "rustpython-codegen"))]
pub(crate) fn validate_ast_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<()> {
    let source_file = SourceFileBuilder::new("<ast>".to_owned(), "".to_owned()).finish();
    let ast = Node::ast_from_object(vm, &source_file, object)?;
    validate::validate_mod(vm, &ast)?;
    Ok(())
}

// The following flags match the values from Include/cpython/compile.h
pub(crate) use crate::vm::compile_mode::{
    PY_CF_ALLOW_INCOMPLETE_INPUT, PY_CF_ALLOW_TOP_LEVEL_AWAIT, PY_CF_DONT_IMPLY_DEDENT,
    PY_CF_IGNORE_COOKIE, PY_CF_ONLY_AST, PY_CF_OPTIMIZED_AST, PY_CF_SOURCE_IS_UTF8,
    PY_CF_TYPE_COMMENTS,
};
