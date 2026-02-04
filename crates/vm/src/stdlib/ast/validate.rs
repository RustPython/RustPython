// spell-checker: ignore assignlist ifexp

use super::module::Mod;
use crate::{PyResult, VirtualMachine};
use ruff_python_ast as ast;

fn expr_context_name(ctx: ast::ExprContext) -> &'static str {
    match ctx {
        ast::ExprContext::Load => "Load",
        ast::ExprContext::Store => "Store",
        ast::ExprContext::Del => "Del",
        ast::ExprContext::Invalid => "Invalid",
    }
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
        return Err(vm.new_value_error("comprehension with no generators".to_owned()));
    }
    for comp in gens {
        validate_expr(vm, &comp.target, ast::ExprContext::Store)?;
        validate_expr(vm, &comp.iter, ast::ExprContext::Load)?;
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

fn validate_parameters(vm: &VirtualMachine, params: &ast::Parameters) -> PyResult<()> {
    for param in params
        .posonlyargs
        .iter()
        .chain(&params.args)
        .chain(&params.kwonlyargs)
    {
        if let Some(annotation) = &param.parameter.annotation {
            validate_expr(vm, annotation, ast::ExprContext::Load)?;
        }
        if let Some(default) = &param.default {
            validate_expr(vm, default, ast::ExprContext::Load)?;
        }
    }
    if let Some(vararg) = &params.vararg
        && let Some(annotation) = &vararg.annotation
    {
        validate_expr(vm, annotation, ast::ExprContext::Load)?;
    }
    if let Some(kwarg) = &params.kwarg
        && let Some(annotation) = &kwarg.annotation
    {
        validate_expr(vm, annotation, ast::ExprContext::Load)?;
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

fn validate_body(vm: &VirtualMachine, body: &[ast::Stmt], owner: &'static str) -> PyResult<()> {
    validate_nonempty_seq(vm, body.len(), "body", owner)?;
    validate_stmts(vm, body)
}

fn validate_interpolated_elements<'a>(
    vm: &VirtualMachine,
    elements: impl IntoIterator<Item = ast::InterpolatedStringElementRef<'a>>,
) -> PyResult<()> {
    for element in elements {
        if let ast::InterpolatedStringElementRef::Interpolation(interpolation) = element {
            validate_expr(vm, &interpolation.expression, ast::ExprContext::Load)?;
            if let Some(format_spec) = &interpolation.format_spec {
                for spec_element in &format_spec.elements {
                    if let ast::InterpolatedStringElement::Interpolation(spec_interp) = spec_element
                    {
                        validate_expr(vm, &spec_interp.expression, ast::ExprContext::Load)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_pattern_match_value(vm: &VirtualMachine, expr: &ast::Expr) -> PyResult<()> {
    validate_expr(vm, expr, ast::ExprContext::Load)?;
    match expr {
        ast::Expr::NumberLiteral(_) | ast::Expr::StringLiteral(_) | ast::Expr::BytesLiteral(_) => {
            Ok(())
        }
        ast::Expr::Attribute(_) => Ok(()),
        ast::Expr::UnaryOp(op) => match &*op.operand {
            ast::Expr::NumberLiteral(_) => Ok(()),
            _ => Err(vm.new_value_error(
                "patterns may only match literals and attribute lookups".to_owned(),
            )),
        },
        ast::Expr::BinOp(bin) => match (&*bin.left, &*bin.right) {
            (ast::Expr::NumberLiteral(_), ast::Expr::NumberLiteral(_)) => Ok(()),
            _ => Err(vm.new_value_error(
                "patterns may only match literals and attribute lookups".to_owned(),
            )),
        },
        ast::Expr::FString(_) | ast::Expr::TString(_) => Ok(()),
        ast::Expr::BooleanLiteral(_)
        | ast::Expr::NoneLiteral(_)
        | ast::Expr::EllipsisLiteral(_) => {
            Err(vm.new_value_error("unexpected constant inside of a literal pattern".to_owned()))
        }
        _ => Err(
            vm.new_value_error("patterns may only match literals and attribute lookups".to_owned())
        ),
    }
}

fn validate_capture(vm: &VirtualMachine, name: &ast::Identifier) -> PyResult<()> {
    if name.as_str() == "_" {
        return Err(vm.new_value_error("can't capture name '_' in patterns".to_owned()));
    }
    validate_name(vm, name.id())
}

fn validate_pattern(vm: &VirtualMachine, pattern: &ast::Pattern, star_ok: bool) -> PyResult<()> {
    match pattern {
        ast::Pattern::MatchValue(value) => validate_pattern_match_value(vm, &value.value),
        ast::Pattern::MatchSingleton(singleton) => match singleton.value {
            ast::Singleton::None | ast::Singleton::True | ast::Singleton::False => Ok(()),
        },
        ast::Pattern::MatchSequence(seq) => validate_patterns(vm, &seq.patterns, true),
        ast::Pattern::MatchMapping(mapping) => {
            if mapping.keys.len() != mapping.patterns.len() {
                return Err(vm.new_value_error(
                    "MatchMapping doesn't have the same number of keys as patterns".to_owned(),
                ));
            }
            if let Some(rest) = &mapping.rest {
                validate_capture(vm, rest)?;
            }
            for key in &mapping.keys {
                if let ast::Expr::BooleanLiteral(_) | ast::Expr::NoneLiteral(_) = key {
                    continue;
                }
                validate_pattern_match_value(vm, key)?;
            }
            validate_patterns(vm, &mapping.patterns, false)
        }
        ast::Pattern::MatchClass(match_class) => {
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
                            "MatchClass cls field can only contain Name or Attribute nodes."
                                .to_owned(),
                        ));
                    }
                }
            }
            for keyword in &match_class.arguments.keywords {
                validate_name(vm, keyword.attr.id())?;
            }
            validate_patterns(vm, &match_class.arguments.patterns, false)?;
            for keyword in &match_class.arguments.keywords {
                validate_pattern(vm, &keyword.pattern, false)?;
            }
            Ok(())
        }
        ast::Pattern::MatchStar(star) => {
            if !star_ok {
                return Err(vm.new_value_error("can't use MatchStar here".to_owned()));
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
                            "MatchAs must specify a target name if a pattern is given".to_owned(),
                        ));
                    }
                    validate_pattern(vm, pattern, false)
                }
            }
        }
        ast::Pattern::MatchOr(match_or) => {
            if match_or.patterns.len() < 2 {
                return Err(vm.new_value_error("MatchOr requires at least 2 patterns".to_owned()));
            }
            validate_patterns(vm, &match_or.patterns, false)
        }
    }
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
    type_params: &Option<Box<ast::TypeParams>>,
) -> PyResult<()> {
    if let Some(type_params) = type_params {
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
                return Err(vm.new_value_error("BoolOp with less than 2 values".to_owned()));
            }
            validate_exprs(vm, &op.values, ast::ExprContext::Load, false)
        }
        ast::Expr::Named(named) => {
            if !matches!(&*named.target, ast::Expr::Name(_)) {
                return Err(vm.new_type_error("NamedExpr target must be a Name".to_owned()));
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
            for item in &dict.items {
                if let Some(key) = &item.key {
                    validate_expr(vm, key, ast::ExprContext::Load)?;
                }
                validate_expr(vm, &item.value, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Expr::Set(set) => validate_exprs(vm, &set.elts, ast::ExprContext::Load, false),
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
                return Err(vm.new_value_error("Compare with no comparators".to_owned()));
            }
            if compare.comparators.len() != compare.ops.len() {
                return Err(vm.new_value_error(
                    "Compare has a different number of comparators and operands".to_owned(),
                ));
            }
            validate_exprs(vm, &compare.comparators, ast::ExprContext::Load, false)?;
            validate_expr(vm, &compare.left, ast::ExprContext::Load)
        }
        ast::Expr::Call(call) => {
            validate_expr(vm, &call.func, ast::ExprContext::Load)?;
            validate_exprs(vm, &call.arguments.args, ast::ExprContext::Load, false)?;
            validate_keywords(vm, &call.arguments.keywords)
        }
        ast::Expr::FString(fstring) => validate_interpolated_elements(
            vm,
            fstring
                .value
                .elements()
                .map(ast::InterpolatedStringElementRef::from),
        ),
        ast::Expr::TString(tstring) => validate_interpolated_elements(
            vm,
            tstring
                .value
                .elements()
                .map(ast::InterpolatedStringElementRef::from),
        ),
        ast::Expr::StringLiteral(_)
        | ast::Expr::BytesLiteral(_)
        | ast::Expr::NumberLiteral(_)
        | ast::Expr::BooleanLiteral(_)
        | ast::Expr::NoneLiteral(_)
        | ast::Expr::EllipsisLiteral(_) => Ok(()),
        ast::Expr::Attribute(attr) => validate_expr(vm, &attr.value, ast::ExprContext::Load),
        ast::Expr::Subscript(sub) => {
            validate_expr(vm, &sub.slice, ast::ExprContext::Load)?;
            validate_expr(vm, &sub.value, ast::ExprContext::Load)
        }
        ast::Expr::Starred(star) => validate_expr(vm, &star.value, ctx),
        ast::Expr::Name(_) => Ok(()),
        ast::Expr::List(list) => validate_exprs(vm, &list.elts, ctx, false),
        ast::Expr::Tuple(tuple) => validate_exprs(vm, &tuple.elts, ctx, false),
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
        ast::Expr::IpyEscapeCommand(_) => Ok(()),
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
            validate_body(vm, &func.body, owner)?;
            validate_type_params(vm, &func.type_params)?;
            validate_parameters(vm, &func.parameters)?;
            validate_decorators(vm, &func.decorator_list)?;
            if let Some(returns) = &func.returns {
                validate_expr(vm, returns, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Stmt::ClassDef(class_def) => {
            validate_body(vm, &class_def.body, "ClassDef")?;
            validate_type_params(vm, &class_def.type_params)?;
            if let Some(arguments) = &class_def.arguments {
                validate_exprs(vm, &arguments.args, ast::ExprContext::Load, false)?;
                validate_keywords(vm, &arguments.keywords)?;
            }
            validate_decorators(vm, &class_def.decorator_list)
        }
        ast::Stmt::Return(ret) => {
            if let Some(value) = &ret.value {
                validate_expr(vm, value, ast::ExprContext::Load)?;
            }
            Ok(())
        }
        ast::Stmt::Delete(del) => validate_assignlist(vm, &del.targets, ast::ExprContext::Del),
        ast::Stmt::Assign(assign) => {
            validate_assignlist(vm, &assign.targets, ast::ExprContext::Store)?;
            validate_expr(vm, &assign.value, ast::ExprContext::Load)
        }
        ast::Stmt::AugAssign(assign) => {
            validate_expr(vm, &assign.target, ast::ExprContext::Store)?;
            validate_expr(vm, &assign.value, ast::ExprContext::Load)
        }
        ast::Stmt::AnnAssign(assign) => {
            if assign.simple && !matches!(&*assign.target, ast::Expr::Name(_)) {
                return Err(vm.new_type_error("AnnAssign with simple non-Name target".to_owned()));
            }
            validate_expr(vm, &assign.target, ast::ExprContext::Store)?;
            if let Some(value) = &assign.value {
                validate_expr(vm, value, ast::ExprContext::Load)?;
            }
            validate_expr(vm, &assign.annotation, ast::ExprContext::Load)
        }
        ast::Stmt::TypeAlias(alias) => {
            if !matches!(&*alias.name, ast::Expr::Name(_)) {
                return Err(vm.new_type_error("TypeAlias with non-Name name".to_owned()));
            }
            validate_expr(vm, &alias.name, ast::ExprContext::Store)?;
            validate_type_params(vm, &alias.type_params)?;
            validate_expr(vm, &alias.value, ast::ExprContext::Load)
        }
        ast::Stmt::For(for_stmt) => {
            let owner = if for_stmt.is_async { "AsyncFor" } else { "For" };
            validate_expr(vm, &for_stmt.target, ast::ExprContext::Store)?;
            validate_expr(vm, &for_stmt.iter, ast::ExprContext::Load)?;
            validate_body(vm, &for_stmt.body, owner)?;
            validate_stmts(vm, &for_stmt.orelse)
        }
        ast::Stmt::While(while_stmt) => {
            validate_expr(vm, &while_stmt.test, ast::ExprContext::Load)?;
            validate_body(vm, &while_stmt.body, "While")?;
            validate_stmts(vm, &while_stmt.orelse)
        }
        ast::Stmt::If(if_stmt) => {
            validate_expr(vm, &if_stmt.test, ast::ExprContext::Load)?;
            validate_body(vm, &if_stmt.body, "If")?;
            for clause in &if_stmt.elif_else_clauses {
                if let Some(test) = &clause.test {
                    validate_expr(vm, test, ast::ExprContext::Load)?;
                }
                validate_body(vm, &clause.body, "If")?;
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
            validate_body(vm, &with_stmt.body, owner)
        }
        ast::Stmt::Match(match_stmt) => {
            validate_expr(vm, &match_stmt.subject, ast::ExprContext::Load)?;
            validate_nonempty_seq(vm, match_stmt.cases.len(), "cases", "Match")?;
            for case in &match_stmt.cases {
                validate_pattern(vm, &case.pattern, false)?;
                if let Some(guard) = &case.guard {
                    validate_expr(vm, guard, ast::ExprContext::Load)?;
                }
                validate_body(vm, &case.body, "match_case")?;
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
                return Err(vm.new_value_error("Raise with cause but no exception".to_owned()));
            }
            Ok(())
        }
        ast::Stmt::Try(try_stmt) => {
            let owner = if try_stmt.is_star { "TryStar" } else { "Try" };
            validate_body(vm, &try_stmt.body, owner)?;
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
            for handler in &try_stmt.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                if let Some(type_expr) = &handler.type_ {
                    validate_expr(vm, type_expr, ast::ExprContext::Load)?;
                }
                validate_body(vm, &handler.body, "ExceptHandler")?;
            }
            validate_stmts(vm, &try_stmt.finalbody)?;
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
        ast::Stmt::Pass(_)
        | ast::Stmt::Break(_)
        | ast::Stmt::Continue(_)
        | ast::Stmt::IpyEscapeCommand(_) => Ok(()),
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
        Mod::Module(module) => validate_stmts(vm, &module.body),
        Mod::Interactive(module) => validate_stmts(vm, &module.body),
        Mod::Expression(expr) => validate_expr(vm, &expr.body, ast::ExprContext::Load),
        Mod::FunctionType(func_type) => {
            validate_exprs(vm, &func_type.argtypes, ast::ExprContext::Load, false)?;
            validate_expr(vm, &func_type.returns, ast::ExprContext::Load)
        }
    }
}
