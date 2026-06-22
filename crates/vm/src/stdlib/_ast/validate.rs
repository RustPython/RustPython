// spell-checker: ignore assignlist ifexp

use super::module::Mod;
use crate::{PyResult, VirtualMachine, compiler::CompileError};
use core::cell::RefCell;
use ruff_python_ast as ast;
use rustpython_codegen::error::{CodegenError, CodegenErrorType};
use rustpython_codegen::{
    PublicAstExprList, PublicAstFormattedValue, PublicAstInterpolation, PublicAstNodeMap,
};
use rustpython_compiler_core::bytecode::ConstantData;

type AstConstantOverrides<'a> = Option<&'a PublicAstNodeMap<ConstantData>>;
type AstInterpolationOverrides<'a> = Option<&'a PublicAstNodeMap<PublicAstInterpolation>>;
type AstFormattedValueOverrides<'a> = Option<&'a PublicAstNodeMap<PublicAstFormattedValue>>;
type AstImportFromLevelOverrides<'a> =
    Option<&'a super::constant::PublicAstImportFromLevelOverrideMap>;
type AstInvalidConstantOverrides<'a> =
    Option<&'a super::constant::PublicAstInvalidConstantOverrideMap>;
type AstExprListOverrides<'a> = Option<&'a super::constant::PublicAstExprListOverrideMap>;
type AstPatternListOverrides<'a> = Option<&'a super::constant::PublicAstPatternListOverrideMap>;
type AstExprOptionListOverrides<'a> =
    Option<&'a super::constant::PublicAstExprOptionListOverrideMap>;
type AstExprListFieldOverrides<'a> = Option<&'a super::constant::PublicAstExprListFieldOverrideMap>;
type AstStmtListOverrides<'a> = Option<&'a super::constant::PublicAstStmtListOverrideMap>;
type AstExceptHandlerListOverrides<'a> =
    Option<&'a super::constant::PublicAstExceptHandlerListOverrideMap>;
type AstTypeParamListOverrides<'a> = Option<&'a super::constant::PublicAstTypeParamListOverrideMap>;
type AstMatchClassOverrides<'a> = Option<&'a super::constant::PublicAstMatchClassOverrideMap>;

thread_local! {
    // Validation borrows the same public-AST side tables created in constant.rs;
    // these TLS slots add no new storage policy.
    static PUBLIC_AST_INVALID_CONSTANTS: RefCell<Option<super::constant::PublicAstInvalidConstantOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_JOINED_STRS: RefCell<Option<super::constant::PublicAstExprListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_TEMPLATE_STRS: RefCell<Option<super::constant::PublicAstExprListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_PATTERN_LISTS: RefCell<Option<super::constant::PublicAstPatternListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_EXPR_OPTION_LISTS: RefCell<Option<super::constant::PublicAstExprOptionListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_EXPR_LISTS: RefCell<Option<super::constant::PublicAstExprListFieldOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_STMT_LISTS: RefCell<Option<super::constant::PublicAstStmtListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_EXCEPT_HANDLER_LISTS: RefCell<Option<super::constant::PublicAstExceptHandlerListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_TYPE_PARAM_LISTS: RefCell<Option<super::constant::PublicAstTypeParamListOverrideMap>> = const { RefCell::new(None) };
    static PUBLIC_AST_MATCH_CLASSES: RefCell<Option<super::constant::PublicAstMatchClassOverrideMap>> = const { RefCell::new(None) };
}

fn public_ast_invalid_constant_type(expr: &ast::Expr) -> Option<String> {
    let index = ast::HasNodeIndex::node_index(expr).load();
    if index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_INVALID_CONSTANTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|invalid_constants| invalid_constants.get(&index).cloned())
    })
}

fn public_ast_joined_str_values(expr: &ast::ExprFString) -> Option<PublicAstExprList> {
    let index = expr.node_index.load();
    if index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_JOINED_STRS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|joined_strs| joined_strs.get(&index).cloned())
    })
}

fn public_ast_template_str_values(expr: &ast::ExprTString) -> Option<PublicAstExprList> {
    let index = expr.node_index.load();
    if index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_TEMPLATE_STRS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|template_strs| template_strs.get(&index).cloned())
    })
}

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
        validate_public_expr_list_slots(
            vm,
            comp.node_index.load(),
            super::constant::PublicAstExprListField::Ifs,
            ast::ExprContext::Load,
        )?;
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
    if let Some(defaults) = public_expr_option_list(params.node_index.load()) {
        for default in defaults.values {
            let Some(default) = default else {
                return Err(vm.new_value_error("None disallowed in expression list"));
            };
            validate_expr(vm, &default, ast::ExprContext::Load)?;
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
    node_index: ast::NodeIndex,
    owner: &'static str,
    ast_constant_overrides: AstConstantOverrides<'_>,
    ast_import_from_level_overrides: AstImportFromLevelOverrides<'_>,
) -> PyResult<()> {
    validate_nonempty_seq(vm, body.len(), "body", owner)?;
    validate_public_stmt_list_slots(
        vm,
        node_index,
        super::constant::PublicAstStmtListField::Body,
    )?;
    validate_stmts(
        vm,
        body,
        ast_constant_overrides,
        ast_import_from_level_overrides,
    )
}

fn validate_interpolated_elements<'a>(
    vm: &VirtualMachine,
    elements: impl IntoIterator<Item = ast::InterpolatedStringElementRef<'a>>,
) -> PyResult<()> {
    for element in elements {
        if let ast::InterpolatedStringElementRef::Interpolation(interpolation) = element {
            validate_expr(vm, &interpolation.expression, ast::ExprContext::Load)?;
            if let Some(format_spec) = &interpolation.format_spec {
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

fn public_ast_constant_override<'a>(
    overrides: AstConstantOverrides<'a>,
    expr: &ast::Expr,
) -> Option<&'a ConstantData> {
    let index = ast::HasNodeIndex::node_index(expr).load();
    if index == ast::NodeIndex::NONE {
        return None;
    }
    overrides?.get(&index)
}

fn validate_pattern_match_value(
    vm: &VirtualMachine,
    expr: &ast::Expr,
    ast_constant_overrides: AstConstantOverrides<'_>,
) -> PyResult<()> {
    validate_expr(vm, expr, ast::ExprContext::Load)?;
    if let Some(constant) = public_ast_constant_override(ast_constant_overrides, expr) {
        return match constant {
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

fn validate_pattern(
    vm: &VirtualMachine,
    pattern: &ast::Pattern,
    star_ok: bool,
    ast_constant_overrides: AstConstantOverrides<'_>,
) -> PyResult<()> {
    match pattern {
        ast::Pattern::MatchValue(value) => {
            validate_pattern_match_value(vm, &value.value, ast_constant_overrides)
        }
        ast::Pattern::MatchSingleton(singleton) => match singleton.value {
            ast::Singleton::None | ast::Singleton::True | ast::Singleton::False => Ok(()),
        },
        ast::Pattern::MatchSequence(seq) => {
            validate_public_pattern_list_slots(vm, seq.node_index.load())?;
            validate_patterns(vm, &seq.patterns, true, ast_constant_overrides)
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
            validate_public_expr_option_list_slots(vm, mapping.node_index.load())?;
            for key in &mapping.keys {
                if let ast::Expr::BooleanLiteral(_) | ast::Expr::NoneLiteral(_) = key {
                    continue;
                }
                validate_pattern_match_value(vm, key, ast_constant_overrides)?;
            }
            validate_public_pattern_list_slots(vm, mapping.node_index.load())?;
            validate_patterns(vm, &mapping.patterns, false, ast_constant_overrides)
        }
        ast::Pattern::MatchClass(match_class) => {
            let public_match_class = public_match_class(match_class.node_index.load());
            if let Some(values) = &public_match_class
                && values.kwd_attrs.len() != values.kwd_patterns.len()
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
            if let Some(values) = &public_match_class {
                validate_public_nullable_patterns(vm, &values.patterns)?;
            }
            validate_patterns(
                vm,
                &match_class.arguments.patterns,
                false,
                ast_constant_overrides,
            )?;
            if let Some(values) = &public_match_class {
                validate_public_nullable_patterns(vm, &values.kwd_patterns)?;
            }
            for keyword in &match_class.arguments.keywords {
                validate_pattern(vm, &keyword.pattern, false, ast_constant_overrides)?;
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
                    validate_pattern(vm, pattern, false, ast_constant_overrides)
                }
            }
        }
        ast::Pattern::MatchOr(match_or) => {
            if match_or.patterns.len() < 2 {
                return Err(vm.new_value_error("MatchOr requires at least 2 patterns"));
            }
            validate_public_pattern_list_slots(vm, match_or.node_index.load())?;
            validate_patterns(vm, &match_or.patterns, false, ast_constant_overrides)
        }
    }
}

fn public_pattern_list_has_null(node_index: ast::NodeIndex) -> bool {
    if node_index == ast::NodeIndex::NONE {
        return false;
    }
    PUBLIC_AST_PATTERN_LISTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index))
            .is_some_and(|values| values.values.iter().any(Option::is_none))
    })
}

fn validate_public_pattern_list_slots(
    vm: &VirtualMachine,
    node_index: ast::NodeIndex,
) -> PyResult<()> {
    if public_pattern_list_has_null(node_index) {
        return Err(vm.new_value_error("unexpected pattern"));
    }
    Ok(())
}

fn public_expr_option_list_has_null(node_index: ast::NodeIndex) -> bool {
    if node_index == ast::NodeIndex::NONE {
        return false;
    }
    PUBLIC_AST_EXPR_OPTION_LISTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index))
            .is_some_and(|values| values.values.iter().any(Option::is_none))
    })
}

fn public_expr_option_list(
    node_index: ast::NodeIndex,
) -> Option<super::constant::PublicAstExprOptionList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_EXPR_OPTION_LISTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

fn public_expr_list(
    node_index: ast::NodeIndex,
    field: super::constant::PublicAstExprListField,
) -> Option<super::constant::PublicAstExprOptionList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_EXPR_LISTS.with(|cell| {
        cell.borrow().as_ref().and_then(|values| {
            values
                .get(&node_index)
                .and_then(|values| values.get(field))
                .cloned()
        })
    })
}

fn public_stmt_list(
    node_index: ast::NodeIndex,
    field: super::constant::PublicAstStmtListField,
) -> Option<super::constant::PublicAstStmtList> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_STMT_LISTS.with(|cell| {
        cell.borrow().as_ref().and_then(|values| {
            values
                .get(&node_index)
                .and_then(|values| values.get(field))
                .cloned()
        })
    })
}

fn validate_public_expr_option_list_slots(
    vm: &VirtualMachine,
    node_index: ast::NodeIndex,
) -> PyResult<()> {
    if public_expr_option_list_has_null(node_index) {
        return Err(vm.new_value_error("None disallowed in expression list"));
    }
    Ok(())
}

fn validate_public_expr_list_slots(
    vm: &VirtualMachine,
    node_index: ast::NodeIndex,
    field: super::constant::PublicAstExprListField,
    ctx: ast::ExprContext,
) -> PyResult<()> {
    if let Some(values) = public_expr_list(node_index, field) {
        for value in values.values {
            let Some(value) = value else {
                return Err(vm.new_value_error("None disallowed in expression list"));
            };
            validate_expr(vm, &value, ctx)?;
        }
    }
    Ok(())
}

fn validate_public_stmt_list_slots(
    vm: &VirtualMachine,
    node_index: ast::NodeIndex,
    field: super::constant::PublicAstStmtListField,
) -> PyResult<()> {
    if let Some(values) = public_stmt_list(node_index, field)
        && values.values.iter().any(Option::is_none)
    {
        return Err(vm.new_value_error("None disallowed in statement list"));
    }
    Ok(())
}

fn public_except_handler_list_has_null(node_index: ast::NodeIndex) -> bool {
    if node_index == ast::NodeIndex::NONE {
        return false;
    }
    PUBLIC_AST_EXCEPT_HANDLER_LISTS.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index))
            .is_some_and(|values| values.values.iter().any(Option::is_none))
    })
}

fn validate_public_except_handler_list_slots(
    vm: &VirtualMachine,
    node_index: ast::NodeIndex,
) -> PyResult<()> {
    if public_except_handler_list_has_null(node_index) {
        return Err(vm.new_value_error("unexpected excepthandler"));
    }
    Ok(())
}

fn public_match_class(node_index: ast::NodeIndex) -> Option<super::constant::PublicAstMatchClass> {
    if node_index == ast::NodeIndex::NONE {
        return None;
    }
    PUBLIC_AST_MATCH_CLASSES.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|values| values.get(&node_index).cloned())
    })
}

fn validate_public_nullable_patterns(
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
    ast_constant_overrides: AstConstantOverrides<'_>,
) -> PyResult<()> {
    for pattern in patterns {
        validate_pattern(vm, pattern, star_ok, ast_constant_overrides)?;
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
        let node_index = type_params.node_index.load();
        if node_index != ast::NodeIndex::NONE
            && let Some(values) = PUBLIC_AST_TYPE_PARAM_LISTS.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|values| values.get(&node_index).cloned())
            })
        {
            for tp in values.values.iter().flatten() {
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
            validate_public_expr_list_slots(
                vm,
                op.node_index.load(),
                super::constant::PublicAstExprListField::Values,
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
            validate_public_expr_list_slots(
                vm,
                dict.node_index.load(),
                super::constant::PublicAstExprListField::Values,
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
            validate_public_expr_list_slots(
                vm,
                set.node_index.load(),
                super::constant::PublicAstExprListField::Elts,
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
            let Some(key) = &dict.key else {
                return Err(vm.new_value_error("DictComp with no key"));
            };
            validate_expr(vm, key, ast::ExprContext::Load)?;
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
            validate_public_expr_list_slots(
                vm,
                compare.node_index.load(),
                super::constant::PublicAstExprListField::Comparators,
                ast::ExprContext::Load,
            )?;
            validate_exprs(vm, &compare.comparators, ast::ExprContext::Load, false)?;
            validate_expr(vm, &compare.left, ast::ExprContext::Load)
        }
        ast::Expr::Call(call) => {
            validate_expr(vm, &call.func, ast::ExprContext::Load)?;
            validate_public_expr_list_slots(
                vm,
                call.arguments.node_index.load(),
                super::constant::PublicAstExprListField::Args,
                ast::ExprContext::Load,
            )?;
            validate_exprs(vm, &call.arguments.args, ast::ExprContext::Load, false)?;
            validate_keywords(vm, &call.arguments.keywords)
        }
        ast::Expr::FString(fstring) => {
            validate_public_expr_list_slots(
                vm,
                fstring.node_index.load(),
                super::constant::PublicAstExprListField::Values,
                ast::ExprContext::Load,
            )?;
            if let Some(joined_str) = public_ast_joined_str_values(fstring) {
                validate_exprs(vm, &joined_str.values, ast::ExprContext::Load, false)
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
            validate_public_expr_list_slots(
                vm,
                tstring.node_index.load(),
                super::constant::PublicAstExprListField::Values,
                ast::ExprContext::Load,
            )?;
            if let Some(template_str) = public_ast_template_str_values(tstring) {
                validate_exprs(vm, &template_str.values, ast::ExprContext::Load, false)
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
        | ast::Expr::BooleanLiteral(_)
        | ast::Expr::NoneLiteral(_)
        | ast::Expr::EllipsisLiteral(_) => {
            if let Some(invalid_type) = public_ast_invalid_constant_type(expr) {
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
            validate_public_expr_list_slots(
                vm,
                list.node_index.load(),
                super::constant::PublicAstExprListField::Elts,
                ctx,
            )?;
            validate_exprs(vm, &list.elts, ctx, false)
        }
        ast::Expr::Tuple(tuple) => {
            validate_public_expr_list_slots(
                vm,
                tuple.node_index.load(),
                super::constant::PublicAstExprListField::Elts,
                ctx,
            )?;
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

fn validate_stmt(
    vm: &VirtualMachine,
    stmt: &ast::Stmt,
    ast_constant_overrides: AstConstantOverrides<'_>,
    ast_import_from_level_overrides: AstImportFromLevelOverrides<'_>,
) -> PyResult<()> {
    match stmt {
        ast::Stmt::FunctionDef(func) => {
            let owner = if func.is_async {
                "AsyncFunctionDef"
            } else {
                "FunctionDef"
            };
            validate_body(
                vm,
                &func.body,
                func.node_index.load(),
                owner,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
            validate_type_params(vm, func.type_params.as_deref())?;
            validate_parameters(vm, &func.parameters)?;
            validate_public_expr_list_slots(
                vm,
                func.node_index.load(),
                super::constant::PublicAstExprListField::DecoratorList,
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
                class_def.node_index.load(),
                "ClassDef",
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
            validate_type_params(vm, class_def.type_params.as_deref())?;
            if let Some(arguments) = &class_def.arguments {
                validate_public_expr_list_slots(
                    vm,
                    arguments.node_index.load(),
                    super::constant::PublicAstExprListField::Bases,
                    ast::ExprContext::Load,
                )?;
                validate_exprs(vm, &arguments.args, ast::ExprContext::Load, false)?;
                validate_keywords(vm, &arguments.keywords)?;
            }
            validate_public_expr_list_slots(
                vm,
                class_def.node_index.load(),
                super::constant::PublicAstExprListField::DecoratorList,
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
            validate_public_expr_list_slots(
                vm,
                del.node_index.load(),
                super::constant::PublicAstExprListField::Targets,
                ast::ExprContext::Del,
            )?;
            validate_assignlist(vm, &del.targets, ast::ExprContext::Del)
        }
        ast::Stmt::Assign(assign) => {
            validate_public_expr_list_slots(
                vm,
                assign.node_index.load(),
                super::constant::PublicAstExprListField::Targets,
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
            validate_body(
                vm,
                &for_stmt.body,
                for_stmt.node_index.load(),
                owner,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
            validate_public_stmt_list_slots(
                vm,
                for_stmt.node_index.load(),
                super::constant::PublicAstStmtListField::Orelse,
            )?;
            validate_stmts(
                vm,
                &for_stmt.orelse,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )
        }
        ast::Stmt::While(while_stmt) => {
            validate_expr(vm, &while_stmt.test, ast::ExprContext::Load)?;
            validate_body(
                vm,
                &while_stmt.body,
                while_stmt.node_index.load(),
                "While",
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
            validate_public_stmt_list_slots(
                vm,
                while_stmt.node_index.load(),
                super::constant::PublicAstStmtListField::Orelse,
            )?;
            validate_stmts(
                vm,
                &while_stmt.orelse,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )
        }
        ast::Stmt::If(if_stmt) => {
            validate_expr(vm, &if_stmt.test, ast::ExprContext::Load)?;
            validate_body(
                vm,
                &if_stmt.body,
                if_stmt.node_index.load(),
                "If",
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
            validate_public_stmt_list_slots(
                vm,
                if_stmt.node_index.load(),
                super::constant::PublicAstStmtListField::Orelse,
            )?;
            for clause in &if_stmt.elif_else_clauses {
                if let Some(test) = &clause.test {
                    validate_expr(vm, test, ast::ExprContext::Load)?;
                }
                validate_body(
                    vm,
                    &clause.body,
                    clause.node_index.load(),
                    "If",
                    ast_constant_overrides,
                    ast_import_from_level_overrides,
                )?;
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
            validate_body(
                vm,
                &with_stmt.body,
                with_stmt.node_index.load(),
                owner,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )
        }
        ast::Stmt::Match(match_stmt) => {
            validate_expr(vm, &match_stmt.subject, ast::ExprContext::Load)?;
            validate_nonempty_seq(vm, match_stmt.cases.len(), "cases", "Match")?;
            for case in &match_stmt.cases {
                validate_pattern(vm, &case.pattern, false, ast_constant_overrides)?;
                if let Some(guard) = &case.guard {
                    validate_expr(vm, guard, ast::ExprContext::Load)?;
                }
                validate_body(
                    vm,
                    &case.body,
                    case.node_index.load(),
                    "match_case",
                    ast_constant_overrides,
                    ast_import_from_level_overrides,
                )?;
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
            validate_body(
                vm,
                &try_stmt.body,
                try_stmt.node_index.load(),
                owner,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
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
            validate_public_except_handler_list_slots(vm, try_stmt.node_index.load())?;
            for handler in &try_stmt.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                if let Some(type_expr) = &handler.type_ {
                    validate_expr(vm, type_expr, ast::ExprContext::Load)?;
                }
                validate_body(
                    vm,
                    &handler.body,
                    handler.node_index.load(),
                    "ExceptHandler",
                    ast_constant_overrides,
                    ast_import_from_level_overrides,
                )?;
            }
            validate_public_stmt_list_slots(
                vm,
                try_stmt.node_index.load(),
                super::constant::PublicAstStmtListField::FinalBody,
            )?;
            validate_stmts(
                vm,
                &try_stmt.finalbody,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )?;
            validate_public_stmt_list_slots(
                vm,
                try_stmt.node_index.load(),
                super::constant::PublicAstStmtListField::Orelse,
            )?;
            validate_stmts(
                vm,
                &try_stmt.orelse,
                ast_constant_overrides,
                ast_import_from_level_overrides,
            )
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
            if let Some(level) = ast_import_from_level_overrides
                .and_then(|overrides| overrides.get(&import.node_index.load()))
                && *level < 0
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

fn validate_stmts(
    vm: &VirtualMachine,
    stmts: &[ast::Stmt],
    ast_constant_overrides: AstConstantOverrides<'_>,
    ast_import_from_level_overrides: AstImportFromLevelOverrides<'_>,
) -> PyResult<()> {
    for stmt in stmts {
        validate_stmt(
            vm,
            stmt,
            ast_constant_overrides,
            ast_import_from_level_overrides,
        )?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "public AST validation installs independent override tables"
)]
pub(super) fn validate_mod(
    vm: &VirtualMachine,
    module: &Mod,
    ast_constant_overrides: AstConstantOverrides<'_>,
    ast_interpolation_overrides: AstInterpolationOverrides<'_>,
    ast_formatted_value_overrides: AstFormattedValueOverrides<'_>,
    ast_import_from_level_overrides: AstImportFromLevelOverrides<'_>,
    ast_invalid_constant_overrides: AstInvalidConstantOverrides<'_>,
    ast_joined_str_overrides: AstExprListOverrides<'_>,
    ast_template_str_overrides: AstExprListOverrides<'_>,
    ast_pattern_list_overrides: AstPatternListOverrides<'_>,
    ast_expr_option_list_overrides: AstExprOptionListOverrides<'_>,
    ast_expr_list_overrides: AstExprListFieldOverrides<'_>,
    ast_stmt_list_overrides: AstStmtListOverrides<'_>,
    ast_except_handler_list_overrides: AstExceptHandlerListOverrides<'_>,
    ast_type_param_list_overrides: AstTypeParamListOverrides<'_>,
    ast_match_class_overrides: AstMatchClassOverrides<'_>,
) -> PyResult<()> {
    PUBLIC_AST_INVALID_CONSTANTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_invalid_constant_overrides.cloned();
    });
    PUBLIC_AST_JOINED_STRS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_joined_str_overrides.cloned();
    });
    PUBLIC_AST_TEMPLATE_STRS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_template_str_overrides.cloned();
    });
    PUBLIC_AST_PATTERN_LISTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_pattern_list_overrides.cloned();
    });
    PUBLIC_AST_EXPR_OPTION_LISTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_expr_option_list_overrides.cloned();
    });
    PUBLIC_AST_EXPR_LISTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_expr_list_overrides.cloned();
    });
    PUBLIC_AST_STMT_LISTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_stmt_list_overrides.cloned();
    });
    PUBLIC_AST_EXCEPT_HANDLER_LISTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_except_handler_list_overrides.cloned();
    });
    PUBLIC_AST_TYPE_PARAM_LISTS.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_type_param_list_overrides.cloned();
    });
    PUBLIC_AST_MATCH_CLASSES.with(|cell| {
        debug_assert!(cell.borrow().is_none());
        *cell.borrow_mut() = ast_match_class_overrides.cloned();
    });
    let result = (|| {
        if let Some(overrides) = ast_interpolation_overrides {
            for interpolation in overrides.values() {
                if let Some(format_spec) = &interpolation.format_spec {
                    validate_expr(vm, format_spec, ast::ExprContext::Load)?;
                }
            }
        }
        if let Some(overrides) = ast_formatted_value_overrides {
            for formatted_value in overrides.values() {
                if let Some(format_spec) = &formatted_value.format_spec {
                    validate_expr(vm, format_spec, ast::ExprContext::Load)?;
                }
            }
        }
        match module {
            Mod::Module(module) => {
                validate_public_stmt_list_slots(
                    vm,
                    module.module.node_index.load(),
                    super::constant::PublicAstStmtListField::Body,
                )?;
                validate_stmts(
                    vm,
                    &module.module.body,
                    ast_constant_overrides,
                    ast_import_from_level_overrides,
                )
            }
            Mod::Interactive(module) => {
                validate_public_stmt_list_slots(
                    vm,
                    module.node_index.load(),
                    super::constant::PublicAstStmtListField::Body,
                )?;
                validate_stmts(
                    vm,
                    &module.body,
                    ast_constant_overrides,
                    ast_import_from_level_overrides,
                )
            }
            Mod::Expression(expr) => validate_expr(vm, &expr.body, ast::ExprContext::Load),
            Mod::FunctionType(func_type) => {
                validate_public_expr_option_list_slots(vm, func_type.node_index.load())?;
                validate_exprs(vm, &func_type.argtypes, ast::ExprContext::Load, false)?;
                validate_expr(vm, &func_type.returns, ast::ExprContext::Load)
            }
        }
    })();
    PUBLIC_AST_INVALID_CONSTANTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_JOINED_STRS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_TEMPLATE_STRS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_PATTERN_LISTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_EXPR_OPTION_LISTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_EXPR_LISTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_STMT_LISTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_EXCEPT_HANDLER_LISTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_TYPE_PARAM_LISTS.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    PUBLIC_AST_MATCH_CLASSES.with(|cell| {
        let _ = cell.borrow_mut().take();
    });
    result
}
