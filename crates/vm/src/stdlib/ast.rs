//! `ast` standard module for abstract syntax trees.

//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

pub(crate) use python::_ast::module_def;

mod pyast;

use crate::builtins::{PyInt, PyStr};
use crate::stdlib::ast::module::{Mod, ModFunctionType, ModInteractive};
use crate::stdlib::ast::node::BoxedSlice;
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

    if start_row_val > end_row_val {
        return Err(vm.new_value_error(format!(
            "AST node line range ({}, {}) is not valid",
            start_row_val, end_row_val
        )));
    }
    if (start_row_val < 0 && end_row_val != start_row_val)
        || (start_col_val < 0 && end_col_val != start_col_val)
    {
        return Err(vm.new_value_error(format!(
            "AST node column range ({}, {}) for line range ({}, {}) is not valid",
            start_col_val, end_col_val, start_row_val, end_row_val
        )));
    }
    if start_row_val == end_row_val && start_col_val > end_col_val {
        return Err(vm.new_value_error(format!(
            "line {}, column {}-{} is not a valid range",
            start_row_val, start_col_val, end_col_val
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

/// Return the expected AST mod type class for a compile() mode string.
pub(crate) fn mode_type_and_name(
    ctx: &Context,
    mode: &str,
) -> Option<(PyRef<PyType>, &'static str)> {
    match mode {
        "exec" => Some((pyast::NodeModModule::make_class(ctx), "Module")),
        "eval" => Some((pyast::NodeModExpression::make_class(ctx), "Expression")),
        "single" => Some((pyast::NodeModInteractive::make_class(ctx), "Interactive")),
        "func_type" => Some((pyast::NodeModFunctionType::make_class(ctx), "FunctionType")),
        _ => None,
    }
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
pub(crate) fn parse(
    vm: &VirtualMachine,
    source: &str,
    mode: parser::Mode,
    optimize: u8,
    target_version: Option<ast::PythonVersion>,
    type_comments: bool,
) -> Result<PyObjectRef, CompileError> {
    let source_file = SourceFileBuilder::new("".to_owned(), source.to_owned()).finish();
    let mut options = parser::ParseOptions::from(mode);
    let target_version = target_version.unwrap_or(ast::PythonVersion::PY314);
    options = options.with_target_version(target_version);
    let parsed = parser::parse(source, options).map_err(|parse_error| {
        let range = text_range_to_source_range(&source_file, parse_error.location);
        ParseError {
            error: parse_error.error,
            raw_location: parse_error.location,
            location: range.start.to_source_location(),
            end_location: range.end.to_source_location(),
            source_path: "<unknown>".to_string(),
        }
    })?;

    if let Some(error) = parsed.unsupported_syntax_errors().first() {
        let range = text_range_to_source_range(&source_file, error.range());
        return Err(ParseError {
            error: parser::ParseErrorType::OtherError(error.to_string()),
            raw_location: error.range(),
            location: range.start.to_source_location(),
            end_location: range.end.to_source_location(),
            source_path: "<unknown>".to_string(),
        }
        .into());
    }

    let mut top = parsed.into_syntax();
    if optimize > 0 {
        fold_match_value_constants(&mut top);
    }
    if optimize >= 2 {
        strip_docstrings(&mut top);
    }
    let top = match top {
        ast::Mod::Module(m) => Mod::Module(m),
        ast::Mod::Expression(e) => Mod::Expression(e),
    };
    let obj = top.ast_to_object(vm, &source_file);
    if type_comments && obj.class().is(pyast::NodeModModule::static_type()) {
        let type_ignores = type_ignores_from_source(vm, source)?;
        let dict = obj.as_object().dict().unwrap();
        dict.set_item("type_ignores", vm.ctx.new_list(type_ignores).into(), vm)
            .unwrap();
    }
    Ok(obj)
}

#[cfg(feature = "parser")]
pub(crate) fn wrap_interactive(vm: &VirtualMachine, module_obj: PyObjectRef) -> PyResult {
    if !module_obj.class().is(pyast::NodeModModule::static_type()) {
        return Err(vm.new_type_error("expected Module node".to_owned()));
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
    let _ = target_version;
    let source = source.trim();
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
        }
        .into());
    };

    let left = source[..split_at].trim();
    let right = source[split_at + 2..].trim();

    let parse_expr = |expr_src: &str| -> Result<ast::Expr, CompileError> {
        let source_file = SourceFileBuilder::new("".to_owned(), expr_src.to_owned()).finish();
        let parsed = parser::parse_expression(expr_src).map_err(|parse_error| {
            let range = text_range_to_source_range(&source_file, parse_error.location);
            ParseError {
                error: parse_error.error,
                raw_location: parse_error.location,
                location: range.start.to_source_location(),
                end_location: range.end.to_source_location(),
                source_path: "<unknown>".to_string(),
            }
        })?;
        Ok(*parsed.into_syntax().body)
    };

    let arg_expr = parse_expr(left)?;
    let returns = parse_expr(right)?;

    let argtypes: Vec<ast::Expr> = match arg_expr {
        ast::Expr::Tuple(tup) => tup.elts,
        ast::Expr::Name(_) | ast::Expr::Subscript(_) | ast::Expr::Attribute(_) => vec![arg_expr],
        other => vec![other],
    };

    let func_type = ModFunctionType {
        argtypes: argtypes.into_boxed_slice(),
        returns,
        range: TextRange::default(),
    };
    let source_file = SourceFileBuilder::new("".to_owned(), source.to_owned()).finish();
    Ok(func_type.ast_to_object(vm, &source_file))
}

fn type_ignores_from_source(
    vm: &VirtualMachine,
    source: &str,
) -> Result<Vec<PyObjectRef>, CompileError> {
    let mut ignores = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let Some(pos) = line.find("#") else {
            continue;
        };
        let comment = &line[pos + 1..];
        let comment = comment.trim_start();
        let Some(rest) = comment.strip_prefix("type: ignore") else {
            continue;
        };
        let tag = rest.trim_start();
        let tag = if tag.is_empty() { "" } else { tag };
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
    Ok(ignores)
}

#[cfg(feature = "parser")]
fn fold_match_value_constants(top: &mut ast::Mod) {
    match top {
        ast::Mod::Module(module) => fold_stmts(&mut module.body),
        ast::Mod::Expression(_expr) => {}
    }
}

#[cfg(feature = "parser")]
fn strip_docstrings(top: &mut ast::Mod) {
    match top {
        ast::Mod::Module(module) => strip_docstring_in_body(&mut module.body),
        ast::Mod::Expression(_expr) => {}
    }
}

#[cfg(feature = "parser")]
fn strip_docstring_in_body(body: &mut Vec<ast::Stmt>) {
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

#[cfg(feature = "parser")]
fn take_docstring(body: &mut Vec<ast::Stmt>) -> Option<TextRange> {
    let ast::Stmt::Expr(expr_stmt) = body.first()? else {
        return None;
    };
    if matches!(expr_stmt.value.as_ref(), ast::Expr::StringLiteral(_)) {
        let range = expr_stmt.range;
        body.remove(0);
        return Some(range);
    }
    None
}

#[cfg(feature = "parser")]
fn fold_stmts(stmts: &mut [ast::Stmt]) {
    for stmt in stmts {
        fold_stmt(stmt);
    }
}

#[cfg(feature = "parser")]
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

#[cfg(feature = "parser")]
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

#[cfg(feature = "parser")]
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

        if let Some(number) = fold_number_binop(&left.value, &binop.op, &right.value) {
            *expr = Expr::NumberLiteral(ast::ExprNumberLiteral {
                node_index: binop.node_index.clone(),
                range: binop.range,
                value: number,
            });
        }
    }
}

#[cfg(feature = "parser")]
fn fold_number_binop(
    left: &ast::Number,
    op: &ast::Operator,
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

#[cfg(feature = "parser")]
fn number_to_complex(number: &ast::Number) -> Option<(f64, f64, bool)> {
    match number {
        ast::Number::Complex { real, imag } => Some((*real, *imag, true)),
        ast::Number::Float(value) => Some((*value, 0.0, false)),
        ast::Number::Int(value) => value.as_i64().map(|value| (value as f64, 0.0, false)),
    }
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
    validate::validate_mod(vm, &ast)?;
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

#[cfg(feature = "codegen")]
pub(crate) fn validate_ast_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<()> {
    let source_file = SourceFileBuilder::new("<ast>".to_owned(), "".to_owned()).finish();
    let ast: Mod = Node::ast_from_object(vm, &source_file, object)?;
    validate::validate_mod(vm, &ast)?;
    Ok(())
}

// Used by builtins::compile()
pub const PY_CF_ONLY_AST: i32 = 0x0400;

// The following flags match the values from Include/cpython/compile.h
// Caveat emptor: These flags are undocumented on purpose and depending
// on their effect outside the standard library is **unsupported**.
pub const PY_CF_SOURCE_IS_UTF8: i32 = 0x0100;
pub const PY_CF_DONT_IMPLY_DEDENT: i32 = 0x200;
pub const PY_CF_IGNORE_COOKIE: i32 = 0x0800;
pub const PY_CF_ALLOW_INCOMPLETE_INPUT: i32 = 0x4000;
pub const PY_CF_OPTIMIZED_AST: i32 = 0x8000 | PY_CF_ONLY_AST;
pub const PY_CF_TYPE_COMMENTS: i32 = 0x1000;
pub const PY_CF_ALLOW_TOP_LEVEL_AWAIT: i32 = 0x2000;

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
pub const PY_COMPILE_FLAGS_MASK: i32 = PY_CF_ONLY_AST
    | PY_CF_SOURCE_IS_UTF8
    | PY_CF_DONT_IMPLY_DEDENT
    | PY_CF_IGNORE_COOKIE
    | PY_CF_ALLOW_TOP_LEVEL_AWAIT
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
