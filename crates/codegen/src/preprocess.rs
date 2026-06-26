use alloc::{boxed::Box, string::String, vec::Vec};

use ruff_python_ast::{
    self as ast, AtomicNodeIndex, ConversionFlag, Expr, ExprFString, FString, FStringFlags,
    FStringValue, HasNodeIndex, InterpolatedElement, InterpolatedStringElement,
    InterpolatedStringElements, InterpolatedStringFormatSpec, InterpolatedStringLiteralElement,
    Operator,
    visitor::transformer::{self, Transformer},
};
use ruff_text_size::{Ranged, TextRange};
use rustpython_compiler_core::bytecode;

const MAXDIGITS: usize = 3;
const F_LJUST: u8 = 1;

/// ast_preprocess.c ControlFlowInFinallyContext
#[derive(Clone, Copy)]
struct ControlFlowInFinallyContext {
    in_finally: bool,
    in_funcdef: bool,
    in_loop: bool,
}

/// ast_preprocess.c before_return
fn before_return<E>(
    contexts: &[ControlFlowInFinallyContext],
    range: TextRange,
    warn: &mut impl FnMut(TextRange, String) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(ctx) = contexts.last()
        && ctx.in_finally
        && !ctx.in_funcdef
    {
        warn(range, "'return' in a 'finally' block".to_owned())?;
    }
    Ok(())
}

/// ast_preprocess.c before_loop_exit
fn before_loop_exit<E>(
    contexts: &[ControlFlowInFinallyContext],
    range: TextRange,
    kw: &str,
    warn: &mut impl FnMut(TextRange, String) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(ctx) = contexts.last()
        && ctx.in_finally
        && !ctx.in_loop
    {
        warn(range, format!("'{kw}' in a 'finally' block"))?;
    }
    Ok(())
}

fn visit_body_with_control_flow_context<E>(
    body: &[ast::Stmt],
    contexts: &mut Vec<ControlFlowInFinallyContext>,
    warn: &mut impl FnMut(TextRange, String) -> Result<(), E>,
    in_finally: bool,
    in_funcdef: bool,
    in_loop: bool,
) -> Result<(), E> {
    contexts.push(ControlFlowInFinallyContext {
        in_finally,
        in_funcdef,
        in_loop,
    });
    visit_body_for_control_flow_in_finally(body, contexts, warn)?;
    contexts.pop();
    Ok(())
}

fn visit_body_for_control_flow_in_finally<E>(
    body: &[ast::Stmt],
    contexts: &mut Vec<ControlFlowInFinallyContext>,
    warn: &mut impl FnMut(TextRange, String) -> Result<(), E>,
) -> Result<(), E> {
    for stmt in body {
        visit_stmt_for_control_flow_in_finally(stmt, contexts, warn)?;
    }
    Ok(())
}

/// ast_preprocess.c astfold_stmt control-flow warning traversal.
fn visit_stmt_for_control_flow_in_finally<E>(
    stmt: &ast::Stmt,
    contexts: &mut Vec<ControlFlowInFinallyContext>,
    warn: &mut impl FnMut(TextRange, String) -> Result<(), E>,
) -> Result<(), E> {
    match stmt {
        ast::Stmt::FunctionDef(function) => {
            visit_body_with_control_flow_context(
                &function.body,
                contexts,
                warn,
                false,
                true,
                false,
            )?;
        }
        ast::Stmt::ClassDef(class) => {
            visit_body_for_control_flow_in_finally(&class.body, contexts, warn)?;
        }
        ast::Stmt::Return(return_stmt) => {
            before_return(contexts, return_stmt.range, warn)?;
        }
        ast::Stmt::For(for_stmt) => {
            visit_body_with_control_flow_context(
                &for_stmt.body,
                contexts,
                warn,
                false,
                false,
                true,
            )?;
            visit_body_for_control_flow_in_finally(&for_stmt.orelse, contexts, warn)?;
        }
        ast::Stmt::While(while_stmt) => {
            visit_body_with_control_flow_context(
                &while_stmt.body,
                contexts,
                warn,
                false,
                false,
                true,
            )?;
            visit_body_for_control_flow_in_finally(&while_stmt.orelse, contexts, warn)?;
        }
        ast::Stmt::If(if_stmt) => {
            visit_body_for_control_flow_in_finally(&if_stmt.body, contexts, warn)?;
            for clause in &if_stmt.elif_else_clauses {
                visit_body_for_control_flow_in_finally(&clause.body, contexts, warn)?;
            }
        }
        ast::Stmt::Try(try_stmt) => {
            visit_body_for_control_flow_in_finally(&try_stmt.body, contexts, warn)?;
            for handler in &try_stmt.handlers {
                match handler {
                    ast::ExceptHandler::ExceptHandler(handler) => {
                        visit_body_for_control_flow_in_finally(&handler.body, contexts, warn)?;
                    }
                }
            }
            visit_body_for_control_flow_in_finally(&try_stmt.orelse, contexts, warn)?;
            visit_body_with_control_flow_context(
                &try_stmt.finalbody,
                contexts,
                warn,
                true,
                false,
                false,
            )?;
        }
        ast::Stmt::With(with_stmt) => {
            visit_body_for_control_flow_in_finally(&with_stmt.body, contexts, warn)?;
        }
        ast::Stmt::Match(match_stmt) => {
            for case in &match_stmt.cases {
                visit_body_for_control_flow_in_finally(&case.body, contexts, warn)?;
            }
        }
        ast::Stmt::Break(break_stmt) => {
            before_loop_exit(contexts, break_stmt.range, "break", warn)?;
        }
        ast::Stmt::Continue(continue_stmt) => {
            before_loop_exit(contexts, continue_stmt.range, "continue", warn)?;
        }
        _ => {}
    }
    Ok(())
}

/// ast_preprocess.c control_flow_in_finally_warning
pub fn warn_control_flow_in_finally<E>(
    module: &ast::Mod,
    mut warn: impl FnMut(TextRange, String) -> Result<(), E>,
) -> Result<(), E> {
    let mut contexts = Vec::new();
    match module {
        ast::Mod::Module(module) => {
            visit_body_for_control_flow_in_finally(&module.body, &mut contexts, &mut warn)?;
        }
        ast::Mod::Expression(_) => {}
    }
    Ok(())
}

pub fn has_future_annotations(module: &ast::Mod) -> bool {
    future_features(module).contains(bytecode::CodeFlags::FUTURE_ANNOTATIONS)
}

pub fn future_features(module: &ast::Mod) -> bytecode::CodeFlags {
    checked_future_features(module).unwrap_or_else(|err| err.features)
}

pub struct FutureFeatureError {
    pub features: bytecode::CodeFlags,
    pub range: TextRange,
    pub kind: FutureFeatureErrorKind,
}

pub enum FutureFeatureErrorKind {
    InvalidFeature(String),
    InvalidBraces,
}

pub fn checked_future_features(
    module: &ast::Mod,
) -> Result<bytecode::CodeFlags, FutureFeatureError> {
    let ast::Mod::Module(module) = module else {
        return Ok(bytecode::CodeFlags::empty());
    };
    checked_future_features_in_body(&module.body)
}

pub fn checked_future_features_in_body(
    body: &[ast::Stmt],
) -> Result<bytecode::CodeFlags, FutureFeatureError> {
    let mut future_features = bytecode::CodeFlags::empty();
    let mut statements = body.iter();
    if let Some(ast::Stmt::Expr(ast::StmtExpr { value, .. })) = statements.clone().next()
        && string_literal_expr_value(value).is_some()
    {
        statements.next();
    }
    for statement in statements {
        match statement {
            ast::Stmt::ImportFrom(ast::StmtImportFrom {
                module,
                names,
                level,
                ..
            }) if *level == 0 && module.as_ref().map(|id| id.as_str()) == Some("__future__") => {
                for alias in names {
                    match alias.name.as_str() {
                        "nested_scopes" | "generators" | "division" | "absolute_import"
                        | "with_statement" | "print_function" | "unicode_literals"
                        | "generator_stop" => {}
                        "annotations" => {
                            future_features.insert(bytecode::CodeFlags::FUTURE_ANNOTATIONS);
                        }
                        // Accept the future feature name, but leave it
                        // as a RustPython no-op.
                        "barry_as_FLUFL" => {}
                        "braces" => {
                            return Err(FutureFeatureError {
                                features: future_features,
                                range: alias.range,
                                kind: FutureFeatureErrorKind::InvalidBraces,
                            });
                        }
                        other => {
                            return Err(FutureFeatureError {
                                features: future_features,
                                range: alias.range,
                                kind: FutureFeatureErrorKind::InvalidFeature(other.to_owned()),
                            });
                        }
                    }
                }
            }
            _ => return Ok(future_features),
        }
    }
    Ok(future_features)
}

pub fn preprocess_statements(
    body: &mut [ast::Stmt],
    optimize: u8,
    future_annotations: bool,
    syntax_check_only: bool,
) {
    let preprocessor = AstPreprocessor {
        optimize,
        future_annotations,
        constant_folding: !syntax_check_only,
    };
    for stmt in body {
        preprocessor.visit_stmt(stmt);
    }
}

pub fn preprocess_mod(
    module: &mut ast::Mod,
    optimize: u8,
    future_annotations: bool,
    syntax_check_only: bool,
) {
    let preprocessor = AstPreprocessor {
        optimize,
        future_annotations,
        constant_folding: !syntax_check_only,
    };
    match module {
        ast::Mod::Module(module) => preprocessor.visit_astfold_body(&mut module.body),
        ast::Mod::Expression(expr) => preprocessor.visit_expr(&mut expr.body),
    }
}

struct AstPreprocessor {
    optimize: u8,
    future_annotations: bool,
    constant_folding: bool,
}

impl AstPreprocessor {
    fn visit_astfold_body(&self, body: &mut ast::Suite) {
        let mut docstring = body_starts_with_docstring(body);
        if docstring && self.optimize >= 2 {
            remove_docstring_from_body(body);
            docstring = false;
        }

        for stmt in body.iter_mut() {
            self.visit_stmt(stmt);
        }

        if !docstring && body_starts_with_docstring(body) {
            wrap_first_docstring_as_fstring(body);
        }
    }
}

impl Transformer for AstPreprocessor {
    fn visit_stmt(&self, stmt: &mut ast::Stmt) {
        match stmt {
            ast::Stmt::FunctionDef(function) => {
                if let Some(type_params) = &mut function.type_params {
                    self.visit_type_params(type_params);
                }
                self.visit_parameters(&mut function.parameters);
                self.visit_astfold_body(&mut function.body);
                for decorator in &mut function.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(returns) = &mut function.returns {
                    self.visit_annotation(returns);
                }
            }
            ast::Stmt::ClassDef(class) => {
                if let Some(type_params) = &mut class.type_params {
                    self.visit_type_params(type_params);
                }
                if let Some(arguments) = &mut class.arguments {
                    self.visit_arguments(arguments);
                }
                self.visit_astfold_body(&mut class.body);
                for decorator in &mut class.decorator_list {
                    self.visit_decorator(decorator);
                }
            }
            _ => transformer::walk_stmt(self, stmt),
        }
    }

    fn visit_annotation(&self, expr: &mut Expr) {
        if !self.future_annotations {
            transformer::walk_annotation(self, expr);
        }
    }

    fn visit_pattern(&self, pattern: &mut ast::Pattern) {
        transformer::walk_pattern(self, pattern);
        if !self.constant_folding {
            return;
        }
        match pattern {
            ast::Pattern::MatchValue(value) => fold_match_value_constant_expr(&mut value.value),
            ast::Pattern::MatchMapping(mapping) => {
                for key in &mut mapping.keys {
                    fold_match_value_constant_expr(key);
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        if self.constant_folding {
            if let Some(optimized) = optimize_format(expr) {
                *expr = optimized;
            } else if let Some(optimized) = fold_debug_constant(expr, self.optimize) {
                *expr = optimized;
            }
        }
    }
}

fn fold_debug_constant(expr: &Expr, optimize: u8) -> Option<Expr> {
    let Expr::Name(name) = expr else {
        return None;
    };
    if !matches!(name.ctx, ast::ExprContext::Load) || name.id.as_str() != "__debug__" {
        return None;
    }

    Some(Expr::BooleanLiteral(ast::ExprBooleanLiteral {
        node_index: name.node_index.clone(),
        range: name.range,
        value: optimize == 0,
    }))
}

fn optimize_format(expr: &Expr) -> Option<Expr> {
    let Expr::BinOp(binop) = expr else {
        return None;
    };
    if !matches!(binop.op, Operator::Mod) {
        return None;
    }
    let (format, _) = string_literal_expr_value(&binop.left)?;
    let Expr::Tuple(tuple) = binop.right.as_ref() else {
        return None;
    };
    if tuple
        .elts
        .iter()
        .any(|expr| matches!(expr, Expr::Starred(_)))
    {
        return None;
    }

    let elements = parse_format(format, &tuple.elts)?;
    Some(Expr::FString(ExprFString {
        node_index: binop.node_index.clone(),
        range: binop.range,
        value: FStringValue::single(FString {
            range: binop.range,
            node_index: binop.node_index.clone(),
            elements: InterpolatedStringElements::from(elements),
            flags: FStringFlags::empty(),
        }),
        runtime_joined_str: None,
        runtime_values: None,
    }))
}

fn parse_format(format: &str, args: &[Expr]) -> Option<Vec<InterpolatedStringElement>> {
    let chars: Vec<char> = format.chars().collect();
    let mut elements = Vec::with_capacity(args.len().saturating_mul(2).saturating_add(1));
    let mut pos = 0;
    let mut arg_idx = 0;

    loop {
        if let Some(literal) = parse_literal(&chars, &mut pos) {
            elements.push(literal.into());
        }
        if pos >= chars.len() {
            break;
        }
        if arg_idx >= args.len() {
            return None;
        }
        debug_assert_eq!(chars[pos], '%');
        pos += 1;
        let formatted = parse_format_arg(&chars, &mut pos, args[arg_idx].clone())?;
        elements.push(formatted.into());
        arg_idx += 1;
    }

    (arg_idx == args.len()).then_some(elements)
}

fn parse_literal(chars: &[char], pos: &mut usize) -> Option<InterpolatedStringLiteralElement> {
    let start = *pos;
    let mut has_percents = false;
    while *pos < chars.len() {
        if chars[*pos] != '%' {
            *pos += 1;
        } else if *pos + 1 < chars.len() && chars[*pos + 1] == '%' {
            has_percents = true;
            *pos += 2;
        } else {
            break;
        }
    }
    if *pos == start {
        return None;
    }

    let mut value = String::new();
    let mut i = start;
    while i < *pos {
        if has_percents && chars[i] == '%' && i + 1 < *pos && chars[i + 1] == '%' {
            value.push('%');
            i += 2;
        } else {
            value.push(chars[i]);
            i += 1;
        }
    }

    Some(generated_literal(value))
}

fn parse_format_arg(chars: &[char], pos: &mut usize, arg: Expr) -> Option<InterpolatedElement> {
    let (spec, flags, width, precision) = simple_format_arg_parse(chars, pos)?;
    let conversion = match spec {
        's' => ConversionFlag::Str,
        'r' => ConversionFlag::Repr,
        'a' => ConversionFlag::Ascii,
        _ => return None,
    };

    let mut format_spec = String::new();
    if flags & F_LJUST == 0
        && let Some(width) = width
        && width > 0
    {
        format_spec.push('>');
    }
    if let Some(width) = width {
        format_spec.push_str(&width.to_string());
    }
    if let Some(precision) = precision {
        format_spec.push('.');
        format_spec.push_str(&precision.to_string());
    }

    let range = arg.range();
    let format_spec = (!format_spec.is_empty()).then(|| {
        Box::new(InterpolatedStringFormatSpec {
            range: TextRange::default(),
            node_index: AtomicNodeIndex::NONE,
            elements: InterpolatedStringElements::from(vec![generated_literal(format_spec).into()]),
        })
    });

    Some(InterpolatedElement {
        range,
        node_index: arg.node_index().clone(),
        expression: Box::new(arg),
        debug_text: None,
        conversion,
        format_spec,
        runtime_str: None,
        runtime_interpolation_format_spec: None,
        runtime_formatted_value_format_spec: None,
    })
}

fn simple_format_arg_parse(
    chars: &[char],
    pos: &mut usize,
) -> Option<(char, u8, Option<u16>, Option<u16>)> {
    let mut flags = 0;
    let mut ch = next_char(chars, pos)?;
    loop {
        match ch {
            '-' => flags |= F_LJUST,
            '+' | ' ' | '#' | '0' => {}
            _ => break,
        }
        ch = next_char(chars, pos)?;
    }

    let width = parse_digits(chars, pos, &mut ch)?;
    let precision = if ch == '.' {
        ch = next_char(chars, pos)?;
        Some(parse_digits(chars, pos, &mut ch)?.unwrap_or(0))
    } else {
        None
    };

    Some((ch, flags, width, precision))
}

fn parse_digits(chars: &[char], pos: &mut usize, ch: &mut char) -> Option<Option<u16>> {
    if !ch.is_ascii_digit() {
        return Some(None);
    }

    let mut value = 0u16;
    let mut digits = 0usize;
    while ch.is_ascii_digit() {
        value = value * 10 + (*ch as u16 - b'0' as u16);
        *ch = next_char(chars, pos)?;
        digits += 1;
        if digits >= MAXDIGITS {
            return None;
        }
    }
    Some(Some(value))
}

fn next_char(chars: &[char], pos: &mut usize) -> Option<char> {
    let ch = chars.get(*pos).copied()?;
    *pos += 1;
    Some(ch)
}

fn generated_literal(value: String) -> InterpolatedStringLiteralElement {
    InterpolatedStringLiteralElement {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
        value: value.into_boxed_str(),
    }
}

fn remove_docstring_from_body(body: &mut ast::Suite) {
    if let Some(range) = take_docstring(body) {
        if !body.is_empty() {
            return;
        }
        let start = range.start();
        let pass_range = TextRange::new(start, start + ruff_text_size::TextSize::from(4));
        body.push(ast::Stmt::Pass(ast::StmtPass {
            node_index: Default::default(),
            range: pass_range,
        }));
    }
}

fn take_docstring(body: &mut ast::Suite) -> Option<TextRange> {
    let ast::Stmt::Expr(expr_stmt) = body.first()? else {
        return None;
    };
    if let Some((_, range)) = string_literal_expr_value(&expr_stmt.value) {
        body.remove(0);
        return Some(range);
    }
    None
}

fn body_starts_with_docstring(body: &[ast::Stmt]) -> bool {
    let Some(ast::Stmt::Expr(expr_stmt)) = body.first() else {
        return false;
    };
    string_literal_expr_value(&expr_stmt.value).is_some()
}

fn wrap_first_docstring_as_fstring(body: &mut [ast::Stmt]) {
    let Some(ast::Stmt::Expr(expr_stmt)) = body.first_mut() else {
        return;
    };
    let Some((value, range)) = string_literal_expr_value(&expr_stmt.value) else {
        return;
    };
    let value = value.to_string();
    *expr_stmt.value = ast::Expr::FString(ast::ExprFString {
        node_index: AtomicNodeIndex::NONE,
        range,
        value: FStringValue::single(FString {
            range,
            node_index: AtomicNodeIndex::NONE,
            elements: InterpolatedStringElements::from(vec![InterpolatedStringElement::Literal(
                InterpolatedStringLiteralElement {
                    range,
                    node_index: AtomicNodeIndex::NONE,
                    value: value.into_boxed_str(),
                },
            )]),
            flags: FStringFlags::empty(),
        }),
        runtime_joined_str: None,
        runtime_values: None,
    });
}

fn string_literal_expr_value(expr: &Expr) -> Option<(&str, TextRange)> {
    match expr {
        Expr::StringLiteral(string) => Some((string.value.to_str(), expr.range())),
        Expr::Constant(ast::ExprConstant {
            value: ast::ConstantValue::Str(value),
            ..
        }) => Some((value.as_ref(), expr.range())),
        _ => None,
    }
}

fn fold_match_value_constant_expr(expr: &mut ast::Expr) {
    match expr {
        ast::Expr::UnaryOp(unary)
            if matches!(unary.op, ast::UnaryOp::USub)
                && matches!(unary.operand.as_ref(), ast::Expr::NumberLiteral(_)) =>
        {
            if let Some(number) = negate_match_number(&unary.operand) {
                *expr = ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
                    node_index: unary.node_index.clone(),
                    range: unary.range,
                    value: number,
                });
            }
        }
        ast::Expr::BinOp(binop) if matches!(binop.op, ast::Operator::Add | ast::Operator::Sub) => {
            fold_match_value_constant_expr(&mut binop.left);
            if let Some(number) = fold_match_number_binop(&binop.left, binop.op, &binop.right) {
                *expr = ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
                    node_index: binop.node_index.clone(),
                    range: binop.range,
                    value: number,
                });
            }
        }
        _ => {}
    }
}

fn negate_match_number(expr: &ast::Expr) -> Option<ast::Number> {
    let ast::Expr::NumberLiteral(number) = expr else {
        return None;
    };
    Some(match &number.value {
        ast::Number::Int(value) => {
            if *value == ast::Int::ZERO {
                ast::Number::Int(ast::Int::ZERO)
            } else {
                return None;
            }
        }
        ast::Number::Float(value) => ast::Number::Float(-value),
        ast::Number::Complex { real, imag } => ast::Number::Complex {
            real: -real,
            imag: -imag,
        },
    })
}

fn fold_match_number_binop(
    left: &ast::Expr,
    op: ast::Operator,
    right: &ast::Expr,
) -> Option<ast::Number> {
    let ast::Expr::NumberLiteral(left) = left else {
        return None;
    };
    let ast::Expr::NumberLiteral(right) = right else {
        return None;
    };
    let right = match right.value {
        ast::Number::Complex { real, imag } => (real, imag),
        _ => return None,
    };
    enum MatchNumberLeft {
        Real(f64),
        Complex { real: f64, imag: f64 },
    }
    let left = match &left.value {
        ast::Number::Int(value) => MatchNumberLeft::Real(value.as_i64()? as f64),
        ast::Number::Float(value) => MatchNumberLeft::Real(*value),
        ast::Number::Complex { real, imag } => MatchNumberLeft::Complex {
            real: *real,
            imag: *imag,
        },
    };
    let (real, imag) = match (left, op) {
        (MatchNumberLeft::Real(left), ast::Operator::Add) => (left + right.0, right.1),
        (MatchNumberLeft::Real(left), ast::Operator::Sub) => (left - right.0, -right.1),
        (MatchNumberLeft::Complex { real, imag }, ast::Operator::Add) => {
            (real + right.0, imag + right.1)
        }
        (MatchNumberLeft::Complex { real, imag }, ast::Operator::Sub) => {
            (real - right.0, imag - right.1)
        }
        _ => return None,
    };
    Some(ast::Number::Complex { real, imag })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_match_value(source: &str) -> ast::Expr {
        let parsed = ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            .unwrap()
            .into_syntax();
        let mut module = parsed;
        let future_annotations = has_future_annotations(&module);
        preprocess_mod(&mut module, 0, future_annotations, false);
        let ast::Mod::Module(module) = module else {
            panic!("expected module");
        };
        let [ast::Stmt::Match(match_stmt)] = &module.body[..] else {
            panic!("expected a single match statement");
        };
        let ast::Pattern::MatchValue(value) = &match_stmt.cases[0].pattern else {
            panic!("expected a value pattern");
        };
        *value.value.clone()
    }

    fn preprocess_source(source: &str) -> ast::Mod {
        let mut module = ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            .unwrap()
            .into_syntax();
        let future_annotations = has_future_annotations(&module);
        preprocess_mod(&mut module, 0, future_annotations, false);
        module
    }

    fn preprocess_source_with_optimize(source: &str, optimize: u8) -> ast::Mod {
        let mut module = ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            .unwrap()
            .into_syntax();
        let future_annotations = has_future_annotations(&module);
        preprocess_mod(&mut module, optimize, future_annotations, false);
        module
    }

    fn preprocess_source_syntax_check_only(source: &str, optimize: u8) -> ast::Mod {
        let mut module = ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            .unwrap()
            .into_syntax();
        let future_annotations = has_future_annotations(&module);
        preprocess_mod(&mut module, optimize, future_annotations, true);
        module
    }

    #[test]
    fn folds_match_value_negative_float_in_preprocess() {
        let value = first_match_value(
            "\
match value:
    case -1.5:
        pass
",
        );
        let ast::Expr::NumberLiteral(number) = value else {
            panic!("expected folded number literal, got {value:?}");
        };
        assert!(matches!(number.value, ast::Number::Float(value) if value == -1.5));
    }

    #[test]
    fn folds_match_value_complex_binop_in_preprocess() {
        let value = first_match_value(
            "\
match value:
    case 1 + 2j:
        pass
",
        );
        let ast::Expr::NumberLiteral(number) = value else {
            panic!("expected folded number literal, got {value:?}");
        };
        assert!(
            matches!(number.value, ast::Number::Complex { real, imag } if real == 1.0 && imag == 2.0)
        );
    }

    #[test]
    fn folds_match_value_complex_complex_binop_in_preprocess() {
        let left = ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            value: ast::Number::Complex {
                real: 0.0,
                imag: 1.0,
            },
        });
        let right = ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            value: ast::Number::Complex {
                real: 0.0,
                imag: 2.0,
            },
        });
        let number = fold_match_number_binop(&left, ast::Operator::Add, &right)
            .expect("CPython fold_const_match_patterns() uses PyNumber_Add");
        assert!(
            matches!(number, ast::Number::Complex { real, imag } if real == 0.0 && imag == 3.0)
        );
    }

    #[test]
    fn folds_match_value_real_minus_zero_complex_preserves_negative_zero_in_preprocess() {
        let value = first_match_value(
            "\
match value:
    case 0 - 0j:
        pass
",
        );
        let ast::Expr::NumberLiteral(number) = value else {
            panic!("expected folded number literal, got {value:?}");
        };
        assert!(matches!(number.value, ast::Number::Complex { real, imag }
                if real == 0.0 && imag == 0.0 && imag.is_sign_negative()));
    }

    #[test]
    fn future_annotations_skip_annotation_preprocess_like_cpython() {
        let module = preprocess_source(
            "\
from __future__ import annotations
def f(x: __debug__) -> __debug__:
    pass
y: __debug__
z = __debug__
",
        );
        let ast::Mod::Module(module) = module else {
            panic!("expected module");
        };
        let ast::Stmt::FunctionDef(function) = &module.body[1] else {
            panic!("expected function");
        };
        let annotation = function.parameters.args[0]
            .parameter
            .annotation
            .as_deref()
            .expect("missing parameter annotation");
        assert!(
            matches!(annotation, ast::Expr::Name(name) if name.id.as_str() == "__debug__"),
            "future annotations should skip parameter annotation folding, got {annotation:?}"
        );
        let returns = function
            .returns
            .as_deref()
            .expect("missing return annotation");
        assert!(
            matches!(returns, ast::Expr::Name(name) if name.id.as_str() == "__debug__"),
            "future annotations should skip return annotation folding, got {returns:?}"
        );
        let ast::Stmt::AnnAssign(ann_assign) = &module.body[2] else {
            panic!("expected annotated assignment");
        };
        assert!(
            matches!(ann_assign.annotation.as_ref(), ast::Expr::Name(name) if name.id.as_str() == "__debug__"),
            "future annotations should skip annotated assignment annotation folding, got {:?}",
            ann_assign.annotation
        );
        let ast::Stmt::Assign(assign) = &module.body[3] else {
            panic!("expected assignment");
        };
        assert!(
            matches!(assign.value.as_ref(), ast::Expr::BooleanLiteral(boolean) if boolean.value),
            "non-annotation expression should still fold __debug__, got {:?}",
            assign.value
        );
    }

    #[test]
    fn late_future_annotations_do_not_affect_preprocess_like_cpython() {
        let module = preprocess_source(
            "\
x = 1
from __future__ import annotations
y: __debug__
",
        );
        let ast::Mod::Module(module) = module else {
            panic!("expected module");
        };
        let ast::Stmt::AnnAssign(ann_assign) = &module.body[2] else {
            panic!("expected annotated assignment");
        };
        assert!(
            matches!(ann_assign.annotation.as_ref(), ast::Expr::BooleanLiteral(boolean) if boolean.value),
            "late future import should not disable annotation folding, got {:?}",
            ann_assign.annotation
        );
    }

    #[test]
    fn optimize_two_wraps_new_docstring_after_removing_original() {
        let module = preprocess_source_with_optimize("\"first\"\n\"second\"\n", 2);
        let ast::Mod::Module(module) = module else {
            panic!("expected module");
        };
        let [ast::Stmt::Expr(expr)] = &module.body[..] else {
            panic!("expected only the second statement to remain");
        };
        assert!(
            matches!(expr.value.as_ref(), ast::Expr::FString(_)),
            "CPython wraps the new leading string as JoinedStr so it is not a docstring"
        );
    }

    #[test]
    fn syntax_check_only_disables_constant_folding_but_keeps_docstring_strip() {
        let module = preprocess_source_syntax_check_only("\"doc\"\nvalue = __debug__\n", 2);
        let ast::Mod::Module(module) = module else {
            panic!("expected module");
        };
        assert!(
            matches!(module.body[0], ast::Stmt::Assign(_)),
            "optimize=2 should still strip docstrings in syntax_check_only mode"
        );
        let ast::Stmt::Assign(assign) = &module.body[0] else {
            panic!("expected assignment");
        };
        assert!(
            matches!(assign.value.as_ref(), ast::Expr::Name(name) if name.id.as_str() == "__debug__"),
            "syntax_check_only should skip __debug__ folding, got {:?}",
            assign.value
        );
    }
}
