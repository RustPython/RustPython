// spell-checker: ignore assignlist ifexp

use super::module::Mod;
use crate::{PyResult, VirtualMachine, compiler::CompileError};
use ruff_python_ast as ast;
use rustpython_codegen::error::{CodegenError, CodegenErrorType};
use rustpython_compiler_core::bytecode::ConstantData;

fn expr_context_name(ctx: ast::ExprContext) -> &'static str {
    match ctx {
        ast::ExprContext::Load => "Load",
        ast::ExprContext::Store => "Store",
        ast::ExprContext::Del => "Del",
        ast::ExprContext::Invalid => "Invalid",
    }
}

fn invalid_syntax_error(vm: &VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
    vm.new_syntax_error(
        &CompileError::Codegen(CodegenError {
            location: None,
            error: CodegenErrorType::SyntaxError("invalid syntax".to_owned()),
            source_path: "<unknown>".to_owned(),
        }),
        None,
    )
}

fn validate_name(vm: &VirtualMachine, name: &ast::name::Name) -> PyResult<()> {
    match name.as_str() {
        "None" | "True" | "False" => Err(vm.new_value_error(format!(
            "identifier field can't represent '{}' constant",
            name.as_str()
        ))),
        _ => Ok(()),
    }
}

fn validate_comprehension(vm: &VirtualMachine, gens: &[ast::Comprehension]) -> PyResult<()> {
    if gens.is_empty() {
        return Err(vm.new_value_error("comprehension with no generators"));
    }
    for comp in gens {
        validate_expr(vm, &comp.target, ast::ExprContext::Store)?;
        validate_expr(vm, &comp.iter, ast::ExprContext::Load)?;
        validate_runtime_expr_list_slots(vm, comp.runtime_ifs.as_ref(), ast::ExprContext::Load)?;
        validate_exprs(vm, &comp.ifs, ast::ExprContext::Load, false)?;
    }
    Ok(())
}

fn validate_keywords(vm: &VirtualMachine, keywords: &[ast::Keyword]) -> PyResult<()> {
    for keyword in keywords {
        validate_expr(vm, &keyword.value, ast::ExprContext::Load)?;
    }
    Ok(())
}

fn validate_parameter_annotation(vm: &VirtualMachine, parameter: &ast::Parameter) -> PyResult<()> {
    if let Some(annotation) = &parameter.annotation {
        validate_expr(vm, annotation, ast::ExprContext::Load)?;
    }
    Ok(())
}

fn validate_parameters(vm: &VirtualMachine, params: &ast::Parameters) -> PyResult<()> {
    for param in params.posonlyargs.iter().chain(&params.args) {
        validate_parameter_annotation(vm, &param.parameter)?;
    }
    if let Some(vararg) = &params.vararg
        && let Some(annotation) = &vararg.annotation
    {
        validate_expr(vm, annotation, ast::ExprContext::Load)?;
    }
    for param in &params.kwonlyargs {
        validate_parameter_annotation(vm, &param.parameter)?;
    }
    if let Some(kwarg) = &params.kwarg
        && let Some(annotation) = &kwarg.annotation
    {
        validate_expr(vm, annotation, ast::ExprContext::Load)?;
    }
    if let Some(defaults) = params.runtime_defaults.as_ref() {
        for default in defaults {
            let Some(default) = default else {
                return Err(vm.new_value_error("None disallowed in expression list"));
            };
            validate_expr(vm, default, ast::ExprContext::Load)?;
        }
    } else {
        for param in params.posonlyargs.iter().chain(&params.args) {
            if let Some(default) = &param.default {
                validate_expr(vm, default, ast::ExprContext::Load)?;
            }
        }
    }
    for param in &params.kwonlyargs {
        if let Some(default) = &param.default {
            validate_expr(vm, default, ast::ExprContext::Load)?;
        }
    }
    Ok(())
}

fn validate_nonempty_seq(
    vm: &VirtualMachine,
    len: usize,
    what: &'static str,
    owner: &'static str,
) -> PyResult<()> {
    if len == 0 {
        return Err(vm.new_value_error(format!("empty {what} on {owner}")));
    }
    Ok(())
}

fn validate_assignlist(
    vm: &VirtualMachine,
    targets: &[ast::Expr],
    ctx: ast::ExprContext,
) -> PyResult<()> {
    validate_nonempty_seq(
        vm,
        targets.len(),
        "targets",
        if ctx == ast::ExprContext::Del {
            "Delete"
        } else {
            "Assign"
        },
    )?;
    validate_exprs(vm, targets, ctx, false)
}

fn validate_body(
    vm: &VirtualMachine,
    body: &[ast::Stmt],
    metadata: Option<&Vec<Option<ast::Stmt>>>,
    owner: &'static str,
) -> PyResult<()> {
    validate_nonempty_seq(vm, body.len(), "body", owner)?;
    validate_runtime_stmt_list_slots(vm, metadata)?;
    validate_stmts(vm, body)
}

fn validate_interpolated_elements<'a>(
    vm: &VirtualMachine,
    elements: impl IntoIterator<Item = ast::InterpolatedStringElementRef<'a>>,
) -> PyResult<()> {
    for element in elements {
        if let ast::InterpolatedStringElementRef::Interpolation(interpolation) = element {
            validate_expr(vm, &interpolation.expression, ast::ExprContext::Load)?;
            if let Some(format_spec) = interpolation.runtime_formatted_value_format_spec.as_deref()
            {
                validate_expr(vm, format_spec, ast::ExprContext::Load)?;
            } else if let Some(format_spec) =
                interpolation.runtime_interpolation_format_spec.as_deref()
            {
                validate_expr(vm, format_spec, ast::ExprContext::Load)?;
            } else if let Some(format_spec) = &interpolation.format_spec {
                validate_interpolated_elements(
                    vm,
                    format_spec
                        .elements
                        .iter()
                        .map(ast::InterpolatedStringElementRef::from),
                )?;
            }
        }
    }
    Ok(())
}

fn ensure_literal_number(expr: &ast::Expr, allow_real: bool, allow_imaginary: bool) -> bool {
    let ast::Expr::NumberLiteral(number) = expr else {
        return false;
    };
    match number.value {
        ast::Number::Int(_) | ast::Number::Float(_) => allow_real,
        ast::Number::Complex { .. } => allow_imaginary,
    }
}

fn ensure_literal_negative(expr: &ast::Expr, allow_real: bool, allow_imaginary: bool) -> bool {
    let ast::Expr::UnaryOp(unary) = expr else {
        return false;
    };
    if unary.op != ast::UnaryOp::USub {
        return false;
    }
    ensure_literal_number(&unary.operand, allow_real, allow_imaginary)
}

fn ensure_literal_complex(expr: &ast::Expr) -> bool {
    let ast::Expr::BinOp(bin) = expr else {
        return false;
    };
    if !matches!(bin.op, ast::Operator::Add | ast::Operator::Sub) {
        return false;
    }
    let real_left = ensure_literal_number(&bin.left, true, false)
        || ensure_literal_negative(&bin.left, true, false);
    real_left && ensure_literal_number(&bin.right, false, true)
}

fn ast_constant_value(expr: &ast::Expr) -> Option<ConstantData> {
    expr.as_constant_expr()
        .map(|expr| super::constant::ast_constant_value_to_constant_data(expr.value.clone()))
}

fn validate_pattern_match_value(vm: &VirtualMachine, expr: &ast::Expr) -> PyResult<()> {
    validate_expr(vm, expr, ast::ExprContext::Load)?;
    if let Some(constant) = ast_constant_value(expr) {
        return match &constant {
            ConstantData::Integer { .. }
            | ConstantData::Float { .. }
            | ConstantData::Bytes { .. }
            | ConstantData::Complex { .. }
            | ConstantData::Str { .. } => Ok(()),
            _ => Err(vm.new_value_error("unexpected constant inside of a literal pattern")),
        };
    }
    match expr {
        ast::Expr::NumberLiteral(_) | ast::Expr::StringLiteral(_) | ast::Expr::BytesLiteral(_) => {
            Ok(())
        }
        ast::Expr::Attribute(_) => Ok(()),
        ast::Expr::UnaryOp(_) if ensure_literal_negative(expr, true, true) => Ok(()),
        ast::Expr::BinOp(_) if ensure_literal_complex(expr) => Ok(()),
        ast::Expr::FString(_) | ast::Expr::TString(_) => Ok(()),
        ast::Expr::BooleanLiteral(_)
        | ast::Expr::NoneLiteral(_)
        | ast::Expr::EllipsisLiteral(_) => {
            Err(vm.new_value_error("unexpected constant inside of a literal pattern"))
        }
        _ => Err(vm.new_value_error("patterns may only match literals and attribute lookups")),
    }
}

fn validate_capture(vm: &VirtualMachine, name: &ast::Identifier) -> PyResult<()> {
    if name.as_str() == "_" {
        return Err(vm.new_value_error("can't capture name '_' in patterns"));
    }
    validate_name(vm, name.id())
}

fn validate_pattern(vm: &VirtualMachine, pattern: &ast::Pattern, star_ok: bool) -> PyResult<()> {
    match pattern {
        ast::Pattern::MatchValue(value) => validate_pattern_match_value(vm, &value.value),
        ast::Pattern::MatchSingleton(singleton) => match singleton.value {
            ast::Singleton::None | ast::Singleton::True | ast::Singleton::False => Ok(()),
        },
        ast::Pattern::MatchSequence(seq) => {
            validate_runtime_pattern_list_slots(vm, seq.runtime_patterns.as_ref())?;
            validate_patterns(vm, &seq.patterns, true)
        }
        ast::Pattern::MatchMapping(mapping) => {
            if mapping.keys.len() != mapping.patterns.len() {
                return Err(vm.new_value_error(
                    "MatchMapping doesn't have the same number of keys as patterns",
                ));
            }
            if let Some(rest) = &mapping.rest {
                validate_capture(vm, rest)?;
            }
            validate_runtime_expr_option_list_slots(vm, mapping.runtime_keys.as_ref())?;
            for key in &mapping.keys {
                if matches!(
                    key,
                    ast::Expr::BooleanLiteral(_)
                        | ast::Expr::NoneLiteral(_)
                        | ast::Expr::Constant(ast::ExprConstant {
                            value: ast::ConstantValue::Boolean(_) | ast::ConstantValue::None,
                            ..
                        })
                ) {
                    continue;
                }
                validate_pattern_match_value(vm, key)?;
            }
            validate_runtime_pattern_list_slots(vm, mapping.runtime_patterns.as_ref())?;
            validate_patterns(vm, &mapping.patterns, false)
        }
        ast::Pattern::MatchClass(match_class) => {
            if let (Some(kwd_attrs), Some(kwd_patterns)) = (
                match_class.runtime_kwd_attrs.as_ref(),
                match_class.runtime_kwd_patterns.as_ref(),
            ) && kwd_attrs.len() != kwd_patterns.len()
            {
                return Err(vm.new_value_error(
                    "MatchClass doesn't have the same number of keyword attributes as patterns",
                ));
            }
            validate_expr(vm, &match_class.cls, ast::ExprContext::Load)?;
            let mut cls = match_class.cls.as_ref();
            loop {
                match cls {
                    ast::Expr::Name(_) => break,
                    ast::Expr::Attribute(attr) => {
                        cls = &attr.value;
                    }
                    _ => {
                        return Err(vm.new_value_error(
                            "MatchClass cls field can only contain Name or Attribute nodes.",
                        ));
                    }
                }
            }
            for keyword in &match_class.arguments.keywords {
                validate_name(vm, keyword.attr.id())?;
            }
            if let Some(patterns) = &match_class.runtime_patterns {
                validate_runtime_nullable_patterns(vm, patterns)?;
            }
            validate_patterns(vm, &match_class.arguments.patterns, false)?;
            if let Some(kwd_patterns) = &match_class.runtime_kwd_patterns {
                validate_runtime_nullable_patterns(vm, kwd_patterns)?;
            }
            for keyword in &match_class.arguments.keywords {
                validate_pattern(vm, &keyword.pattern, false)?;
            }
            Ok(())
        }
        ast::Pattern::MatchStar(star) => {
            if !star_ok {
                return Err(vm.new_value_error("can't use MatchStar here"));
            }
            if let Some(name) = &star.name {
                validate_capture(vm, name)?;
            }
            Ok(())
        }
        ast::Pattern::MatchAs(match_as) => {
            if let Some(name) = &match_as.name {
                validate_capture(vm, name)?;
            }
            match &match_as.pattern {
                None => Ok(()),
                Some(pattern) => {
                    if match_as.name.is_none() {
                        return Err(vm.new_value_error(
                            "MatchAs must specify a target name if a pattern is given",
                        ));
                    }
                    validate_pattern(vm, pattern, false)
                }
            }
        }
        ast::Pattern::MatchOr(match_or) => {
            if match_or.patterns.len() < 2 {
                return Err(vm.new_value_error("MatchOr requires at least 2 patterns"));
            }
            validate_runtime_pattern_list_slots(vm, match_or.runtime_patterns.as_ref())?;
            validate_patterns(vm, &match_or.patterns, false)
        }
    }
}

fn validate_runtime_pattern_list_slots(
    vm: &VirtualMachine,
    values: Option<&Vec<Option<ast::Pattern>>>,
) -> PyResult<()> {
    if values.is_some_and(|values| values.iter().any(Option::is_none)) {
        return Err(vm.new_value_error("unexpected pattern"));
    }
    Ok(())
}

fn validate_runtime_expr_option_list_slots(
    vm: &VirtualMachine,
    values: Option<&Vec<Option<ast::Expr>>>,
) -> PyResult<()> {
    if values.is_some_and(|values| values.iter().any(Option::is_none)) {
        return Err(vm.new_value_error("None disallowed in expression list"));
    }
    Ok(())
}

fn validate_runtime_expr_list_slots(
    vm: &VirtualMachine,
    values: Option<&Vec<Option<ast::Expr>>>,
    ctx: ast::ExprContext,
) -> PyResult<()> {
    if let Some(values) = values {
        for value in values {
            let Some(value) = value else {
                return Err(vm.new_value_error("None disallowed in expression list"));
            };
            validate_expr(vm, value, ctx)?;
        }
    }
    Ok(())
}

fn validate_runtime_stmt_list_slots(
    vm: &VirtualMachine,
    values: Option<&Vec<Option<ast::Stmt>>>,
) -> PyResult<()> {
    if let Some(values) = values
        && values.iter().any(Option::is_none)
    {
        return Err(vm.new_value_error("None disallowed in statement list"));
    }
    Ok(())
}

fn validate_runtime_except_handler_list_slots(
    vm: &VirtualMachine,
    values: Option<&Vec<Option<ast::ExceptHandler>>>,
) -> PyResult<()> {
    if values.is_some_and(|values| values.iter().any(Option::is_none)) {
        return Err(vm.new_value_error("unexpected excepthandler"));
    }
    Ok(())
}

fn validate_runtime_nullable_patterns(
    vm: &VirtualMachine,
    patterns: &[Option<ast::Pattern>],
) -> PyResult<()> {
    if patterns.iter().any(Option::is_none) {
        return Err(vm.new_value_error("unexpected pattern"));
    }
    Ok(())
}

fn validate_patterns(
    vm: &VirtualMachine,
    patterns: &[ast::Pattern],
    star_ok: bool,
) -> PyResult<()> {
    for pattern in patterns {
        validate_pattern(vm, pattern, star_ok)?;
    }
    Ok(())
}

fn validate_typeparam(vm: &VirtualMachine, tp: &ast::TypeParam) -> PyResult<()> {
    match tp {
        ast::TypeParam::TypeVar(tp) => {
            validate_name(vm, tp.name.id())?;
            if let Some(bound) = &tp.bound {
                validate_expr(vm, bound, ast::ExprContext::Load)?;
            }
            if let Some(default) = &tp.default {
                validate_expr(vm, default, ast::ExprContext::Load)?;
            }
        }
        ast::TypeParam::ParamSpec(tp) => {
            validate_name(vm, tp.name.id())?;
            if let Some(default) = &tp.default {
                validate_expr(vm, default, ast::ExprContext::Load)?;
            }
        }
        ast::TypeParam::TypeVarTuple(tp) => {
            validate_name(vm, tp.name.id())?;
            if let Some(default) = &tp.default {
                validate_expr(vm, default, ast::ExprContext::Load)?;
            }
        }
    }
    Ok(())
}

fn validate_type_params(
    vm: &VirtualMachine,
    type_params: Option<&ast::TypeParams>,
) -> PyResult<()> {
    if let Some(type_params) = type_params {
        if let Some(values) = type_params.runtime_type_params.as_ref() {
            for tp in values.iter().flatten() {
                validate_typeparam(vm, tp)?;
            }
            return Ok(());
        }
        for tp in &type_params.type_params {
            validate_typeparam(vm, tp)?;
        }
    }
    Ok(())
}

fn validate_exprs(
    vm: &VirtualMachine,
    exprs: &[ast::Expr],
    ctx: ast::ExprContext,
    _null_ok: bool,
) -> PyResult<()> {
    for expr in exprs {
        validate_expr(vm, expr, ctx)?;
    }
    Ok(())
}

fn validate_expr(vm: &VirtualMachine, expr: &ast::Expr, ctx: ast::ExprContext) -> PyResult<()> {
    let mut check_ctx = true;
    let actual_ctx = match expr {
        ast::Expr::Attribute(attr) => attr.ctx,
        ast::Expr::Subscript(sub) => sub.ctx,
        ast::Expr::Starred(star) => star.ctx,
        ast::Expr::Name(name) => {
            validate_name(vm, name.id())?;
            name.ctx
        }
        ast::Expr::List(list) => list.ctx,
        ast::Expr::Tuple(tuple) => tuple.ctx,
        _ => {
            if ctx != ast::ExprContext::Load {
                return Err(vm.new_value_error(format!(
                    "expression which can't be assigned to in {} context",
                    expr_context_name(ctx)
                )));
            }
            check_ctx = false;
            ast::ExprContext::Invalid
        }
    };
    if check_ctx && actual_ctx != ctx {
        return Err(vm.new_value_error(format!(
            "expression must have {} context but has {} instead",
            expr_context_name(ctx),
            expr_context_name(actual_ctx)
        )));
    }

    match expr {
        ast::Expr::BoolOp(op) => {
            if op.values.len() < 2 {
                return Err(vm.new_value_error("BoolOp with less than 2 values"));
            }
            validate_runtime_expr_list_slots(
                vm,
                op.runtime_values.as_ref(),
                ast::ExprContext::Load,
            )?;
            validate_exprs(vm, &op.values, ast::ExprContext::Load, false)
        }
        ast::Expr::Named(named) => {
            if !matches!(&*named.target, ast::Expr::Name(_)) {
                return Err(vm.new_type_error("NamedExpr target must be a Name"));
            }
            validate_expr(vm, &named.value, ast::ExprContext::Load)
        }
        ast::Expr::BinOp(bin) => {
            validate_expr(vm, &bin.left, ast::ExprContext::Load)?;
            validate_expr(vm, &bin.right, ast::ExprContext::Load)
        }
        ast::Expr::UnaryOp(unary) => validate_expr(vm, &unary.operand, ast::ExprContext::Load),
        ast::Expr::Lambda(lambda) => {
            if let Some(parameters) = &lambda.parameters {
                validate_parameters(vm, parameters)?;
            }
            validate_expr(vm, &lambda.body, ast::ExprContext::Load)
        }
        ast::Expr::If(ifexp) => {
            validate_expr(vm, &ifexp.test, ast::ExprContext::Load)?;
            validate_expr(vm, &ifexp.body, ast::ExprContext::Load)?;
            validate_expr(vm, &ifexp.orelse, ast::ExprContext::Load)
        }
        ast::Expr::Dict(dict) => {
            validate_runtime_expr_list_slots(
                vm,
                dict.runtime_values.as_ref(),
                ast::ExprContext::Load,
            )?;
            for item in &dict.items {
                if let Some(key) = &item.key {
                    validate_expr(vm, key, ast::ExprContext::Load)?;
                }
                validate_expr(vm, &item.value, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Expr::Set(set) => {
            validate_runtime_expr_list_slots(
                vm,
                set.runtime_elts.as_ref(),
                ast::ExprContext::Load,
            )?;
            validate_exprs(vm, &set.elts, ast::ExprContext::Load, false)
        }
        ast::Expr::ListComp(list) => {
            validate_comprehension(vm, &list.generators)?;
            validate_expr(vm, &list.elt, ast::ExprContext::Load)
        }
        ast::Expr::SetComp(set) => {
            validate_comprehension(vm, &set.generators)?;
            validate_expr(vm, &set.elt, ast::ExprContext::Load)
        }
        ast::Expr::DictComp(dict) => {
            validate_comprehension(vm, &dict.generators)?;
            validate_expr(vm, &dict.key, ast::ExprContext::Load)?;
            validate_expr(vm, &dict.value, ast::ExprContext::Load)
        }
        ast::Expr::Generator(generator) => {
            validate_comprehension(vm, &generator.generators)?;
            validate_expr(vm, &generator.elt, ast::ExprContext::Load)
        }
        ast::Expr::Yield(yield_expr) => {
            if let Some(value) = &yield_expr.value {
                validate_expr(vm, value, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Expr::YieldFrom(yield_expr) => {
            validate_expr(vm, &yield_expr.value, ast::ExprContext::Load)
        }
        ast::Expr::Await(await_expr) => {
            validate_expr(vm, &await_expr.value, ast::ExprContext::Load)
        }
        ast::Expr::Compare(compare) => {
            if compare.comparators.is_empty() {
                return Err(vm.new_value_error("Compare with no comparators"));
            }
            if compare.comparators.len() != compare.ops.len() {
                return Err(vm.new_value_error(
                    "Compare has a different number of comparators and operands",
                ));
            }
            validate_runtime_expr_list_slots(
                vm,
                compare.runtime_comparators.as_ref(),
                ast::ExprContext::Load,
            )?;
            validate_exprs(vm, &compare.comparators, ast::ExprContext::Load, false)?;
            validate_expr(vm, &compare.left, ast::ExprContext::Load)
        }
        ast::Expr::Call(call) => {
            validate_expr(vm, &call.func, ast::ExprContext::Load)?;
            validate_runtime_expr_list_slots(
                vm,
                call.arguments.runtime_args.as_ref(),
                ast::ExprContext::Load,
            )?;
            validate_exprs(vm, &call.arguments.args, ast::ExprContext::Load, false)?;
            validate_keywords(vm, &call.arguments.keywords)
        }
        ast::Expr::FString(fstring) => {
            validate_runtime_expr_list_slots(
                vm,
                fstring.runtime_values.as_ref(),
                ast::ExprContext::Load,
            )?;
            if let Some(joined_str) = fstring.runtime_joined_str.as_ref() {
                validate_exprs(vm, joined_str, ast::ExprContext::Load, false)
            } else {
                validate_interpolated_elements(
                    vm,
                    fstring
                        .value
                        .elements()
                        .map(ast::InterpolatedStringElementRef::from),
                )
            }
        }
        ast::Expr::TString(tstring) => {
            validate_runtime_expr_list_slots(
                vm,
                tstring.runtime_values.as_ref(),
                ast::ExprContext::Load,
            )?;
            if let Some(template_str) = tstring.runtime_template_str.as_ref() {
                validate_exprs(vm, template_str, ast::ExprContext::Load, false)
            } else {
                validate_interpolated_elements(
                    vm,
                    tstring
                        .value
                        .elements()
                        .map(ast::InterpolatedStringElementRef::from),
                )
            }
        }
        ast::Expr::StringLiteral(_)
        | ast::Expr::BytesLiteral(_)
        | ast::Expr::NumberLiteral(_)
        | ast::Expr::Constant(_)
        | ast::Expr::BooleanLiteral(_)
        | ast::Expr::NoneLiteral(_)
        | ast::Expr::EllipsisLiteral(_) => {
            if let Some(invalid_type) = super::constant::invalid_constant_type(expr) {
                Err(vm.new_type_error(format!("got an invalid type in Constant: {invalid_type}")))
            } else {
                Ok(())
            }
        }
        ast::Expr::Attribute(attr) => validate_expr(vm, &attr.value, ast::ExprContext::Load),
        ast::Expr::Subscript(sub) => {
            validate_expr(vm, &sub.slice, ast::ExprContext::Load)?;
            validate_expr(vm, &sub.value, ast::ExprContext::Load)
        }
        ast::Expr::Starred(star) => validate_expr(vm, &star.value, ctx),
        ast::Expr::Name(_) => Ok(()),
        ast::Expr::List(list) => {
            validate_runtime_expr_list_slots(vm, list.runtime_elts.as_ref(), ctx)?;
            validate_exprs(vm, &list.elts, ctx, false)
        }
        ast::Expr::Tuple(tuple) => {
            validate_runtime_expr_list_slots(vm, tuple.runtime_elts.as_ref(), ctx)?;
            validate_exprs(vm, &tuple.elts, ctx, false)
        }
        ast::Expr::Slice(slice) => {
            if let Some(lower) = &slice.lower {
                validate_expr(vm, lower, ast::ExprContext::Load)?;
            }
            if let Some(upper) = &slice.upper {
                validate_expr(vm, upper, ast::ExprContext::Load)?;
            }
            if let Some(step) = &slice.step {
                validate_expr(vm, step, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Expr::IpyEscapeCommand(_) => Err(invalid_syntax_error(vm)),
    }
}

fn validate_decorators(vm: &VirtualMachine, decorators: &[ast::Decorator]) -> PyResult<()> {
    for decorator in decorators {
        validate_expr(vm, &decorator.expression, ast::ExprContext::Load)?;
    }
    Ok(())
}

fn validate_stmt(vm: &VirtualMachine, stmt: &ast::Stmt) -> PyResult<()> {
    match stmt {
        ast::Stmt::FunctionDef(func) => {
            let owner = if func.is_async {
                "AsyncFunctionDef"
            } else {
                "FunctionDef"
            };
            validate_body(vm, &func.body, func.runtime_body.as_ref(), owner)?;
            validate_type_params(vm, func.type_params.as_deref())?;
            validate_parameters(vm, &func.parameters)?;
            validate_runtime_expr_list_slots(
                vm,
                func.runtime_decorator_list.as_ref(),
                ast::ExprContext::Load,
            )?;
            validate_decorators(vm, &func.decorator_list)?;
            if let Some(returns) = &func.returns {
                validate_expr(vm, returns, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Stmt::ClassDef(class_def) => {
            validate_body(
                vm,
                &class_def.body,
                class_def.runtime_body.as_ref(),
                "ClassDef",
            )?;
            validate_type_params(vm, class_def.type_params.as_deref())?;
            if let Some(arguments) = &class_def.arguments {
                validate_runtime_expr_list_slots(
                    vm,
                    arguments.runtime_bases.as_ref(),
                    ast::ExprContext::Load,
                )?;
                validate_exprs(vm, &arguments.args, ast::ExprContext::Load, false)?;
                validate_keywords(vm, &arguments.keywords)?;
            }
            validate_runtime_expr_list_slots(
                vm,
                class_def.runtime_decorator_list.as_ref(),
                ast::ExprContext::Load,
            )?;
            validate_decorators(vm, &class_def.decorator_list)
        }
        ast::Stmt::Return(ret) => {
            if let Some(value) = &ret.value {
                validate_expr(vm, value, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Stmt::Delete(del) => {
            validate_runtime_expr_list_slots(
                vm,
                del.runtime_targets.as_ref(),
                ast::ExprContext::Del,
            )?;
            validate_assignlist(vm, &del.targets, ast::ExprContext::Del)
        }
        ast::Stmt::Assign(assign) => {
            validate_runtime_expr_list_slots(
                vm,
                assign.runtime_targets.as_ref(),
                ast::ExprContext::Store,
            )?;
            validate_assignlist(vm, &assign.targets, ast::ExprContext::Store)?;
            validate_expr(vm, &assign.value, ast::ExprContext::Load)
        }
        ast::Stmt::AugAssign(assign) => {
            validate_expr(vm, &assign.target, ast::ExprContext::Store)?;
            validate_expr(vm, &assign.value, ast::ExprContext::Load)
        }
        ast::Stmt::AnnAssign(assign) => {
            if assign.simple && !matches!(&*assign.target, ast::Expr::Name(_)) {
                return Err(vm.new_type_error("AnnAssign with simple non-Name target"));
            }
            validate_expr(vm, &assign.target, ast::ExprContext::Store)?;
            if let Some(value) = &assign.value {
                validate_expr(vm, value, ast::ExprContext::Load)?;
            }
            validate_expr(vm, &assign.annotation, ast::ExprContext::Load)
        }
        ast::Stmt::TypeAlias(alias) => {
            if !matches!(&*alias.name, ast::Expr::Name(_)) {
                return Err(vm.new_type_error("TypeAlias with non-Name name"));
            }
            validate_expr(vm, &alias.name, ast::ExprContext::Store)?;
            validate_type_params(vm, alias.type_params.as_deref())?;
            validate_expr(vm, &alias.value, ast::ExprContext::Load)
        }
        ast::Stmt::For(for_stmt) => {
            let owner = if for_stmt.is_async { "AsyncFor" } else { "For" };
            validate_expr(vm, &for_stmt.target, ast::ExprContext::Store)?;
            validate_expr(vm, &for_stmt.iter, ast::ExprContext::Load)?;
            validate_body(vm, &for_stmt.body, for_stmt.runtime_body.as_ref(), owner)?;
            validate_runtime_stmt_list_slots(vm, for_stmt.runtime_orelse.as_ref())?;
            validate_stmts(vm, &for_stmt.orelse)
        }
        ast::Stmt::While(while_stmt) => {
            validate_expr(vm, &while_stmt.test, ast::ExprContext::Load)?;
            validate_body(
                vm,
                &while_stmt.body,
                while_stmt.runtime_body.as_ref(),
                "While",
            )?;
            validate_runtime_stmt_list_slots(vm, while_stmt.runtime_orelse.as_ref())?;
            validate_stmts(vm, &while_stmt.orelse)
        }
        ast::Stmt::If(if_stmt) => {
            validate_expr(vm, &if_stmt.test, ast::ExprContext::Load)?;
            validate_body(vm, &if_stmt.body, if_stmt.runtime_body.as_ref(), "If")?;
            for clause in &if_stmt.elif_else_clauses {
                if let Some(test) = &clause.test {
                    validate_expr(vm, test, ast::ExprContext::Load)?;
                }
                validate_body(vm, &clause.body, clause.runtime_body.as_ref(), "If")?;
                validate_runtime_stmt_list_slots(vm, clause.runtime_orelse.as_ref())?;
            }
            Ok(())
        }
        ast::Stmt::With(with_stmt) => {
            let owner = if with_stmt.is_async {
                "AsyncWith"
            } else {
                "With"
            };
            validate_nonempty_seq(vm, with_stmt.items.len(), "items", owner)?;
            for item in &with_stmt.items {
                validate_expr(vm, &item.context_expr, ast::ExprContext::Load)?;
                if let Some(optional_vars) = &item.optional_vars {
                    validate_expr(vm, optional_vars, ast::ExprContext::Store)?;
                }
            }
            validate_body(vm, &with_stmt.body, with_stmt.runtime_body.as_ref(), owner)
        }
        ast::Stmt::Match(match_stmt) => {
            validate_expr(vm, &match_stmt.subject, ast::ExprContext::Load)?;
            validate_nonempty_seq(vm, match_stmt.cases.len(), "cases", "Match")?;
            for case in &match_stmt.cases {
                validate_pattern(vm, &case.pattern, false)?;
                if let Some(guard) = &case.guard {
                    validate_expr(vm, guard, ast::ExprContext::Load)?;
                }
                validate_body(vm, &case.body, case.runtime_body.as_ref(), "match_case")?;
            }
            Ok(())
        }
        ast::Stmt::Raise(raise) => {
            if let Some(exc) = &raise.exc {
                validate_expr(vm, exc, ast::ExprContext::Load)?;
                if let Some(cause) = &raise.cause {
                    validate_expr(vm, cause, ast::ExprContext::Load)?;
                }
            } else if raise.cause.is_some() {
                return Err(vm.new_value_error("Raise with cause but no exception"));
            }
            Ok(())
        }
        ast::Stmt::Try(try_stmt) => {
            let owner = if try_stmt.is_star { "TryStar" } else { "Try" };
            validate_body(vm, &try_stmt.body, try_stmt.runtime_body.as_ref(), owner)?;
            if try_stmt.handlers.is_empty() && try_stmt.finalbody.is_empty() {
                return Err(vm.new_value_error(format!(
                    "{owner} has neither except handlers nor finalbody"
                )));
            }
            if try_stmt.handlers.is_empty() && !try_stmt.orelse.is_empty() {
                return Err(
                    vm.new_value_error(format!("{owner} has orelse but no except handlers"))
                );
            }
            validate_runtime_except_handler_list_slots(vm, try_stmt.runtime_handlers.as_ref())?;
            for handler in &try_stmt.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                if let Some(type_expr) = &handler.type_ {
                    validate_expr(vm, type_expr, ast::ExprContext::Load)?;
                }
                validate_body(
                    vm,
                    &handler.body,
                    handler.runtime_body.as_ref(),
                    "ExceptHandler",
                )?;
            }
            validate_runtime_stmt_list_slots(vm, try_stmt.runtime_finalbody.as_ref())?;
            validate_stmts(vm, &try_stmt.finalbody)?;
            validate_runtime_stmt_list_slots(vm, try_stmt.runtime_orelse.as_ref())?;
            validate_stmts(vm, &try_stmt.orelse)
        }
        ast::Stmt::Assert(assert_stmt) => {
            validate_expr(vm, &assert_stmt.test, ast::ExprContext::Load)?;
            if let Some(msg) = &assert_stmt.msg {
                validate_expr(vm, msg, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Stmt::Import(import) => {
            validate_nonempty_seq(vm, import.names.len(), "names", "Import")?;
            Ok(())
        }
        ast::Stmt::ImportFrom(import) => {
            if let Some(level) = import.runtime_level
                && level < 0
            {
                return Err(vm.new_value_error("Negative ImportFrom level"));
            }
            validate_nonempty_seq(vm, import.names.len(), "names", "ImportFrom")?;
            Ok(())
        }
        ast::Stmt::Global(global) => {
            validate_nonempty_seq(vm, global.names.len(), "names", "Global")?;
            Ok(())
        }
        ast::Stmt::Nonlocal(nonlocal) => {
            validate_nonempty_seq(vm, nonlocal.names.len(), "names", "Nonlocal")?;
            Ok(())
        }
        ast::Stmt::Expr(expr) => validate_expr(vm, &expr.value, ast::ExprContext::Load),
        ast::Stmt::Pass(_) | ast::Stmt::Break(_) | ast::Stmt::Continue(_) => Ok(()),
        ast::Stmt::IpyEscapeCommand(_) => Err(invalid_syntax_error(vm)),
    }
}

fn validate_stmts(vm: &VirtualMachine, stmts: &[ast::Stmt]) -> PyResult<()> {
    for stmt in stmts {
        validate_stmt(vm, stmt)?;
    }
    Ok(())
}

pub(super) fn validate_mod(vm: &VirtualMachine, module: &Mod) -> PyResult<()> {
    match module {
        Mod::Module(module) => {
            validate_runtime_stmt_list_slots(vm, module.module.runtime_body.as_ref())?;
            validate_stmts(vm, &module.module.body)
        }
        Mod::Interactive(module) => {
            validate_runtime_stmt_list_slots(vm, module.runtime_body.as_ref())?;
            validate_stmts(vm, &module.body)
        }
        Mod::Expression(expr) => validate_expr(vm, &expr.body, ast::ExprContext::Load),
        Mod::FunctionType(func_type) => {
            validate_runtime_expr_option_list_slots(vm, func_type.runtime_argtypes.as_ref())?;
            validate_exprs(vm, &func_type.argtypes, ast::ExprContext::Load, false)?;
            validate_expr(vm, &func_type.returns, ast::ExprContext::Load)
        }
    }
}
