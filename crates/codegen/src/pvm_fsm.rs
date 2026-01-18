use ruff_python_ast::{
    Arguments, Decorator, Expr, ExprAttribute, ExprCall, ExprName, Mod, Parameters, Stmt,
    StmtAssign, StmtExpr, StmtFunctionDef, visitor::{Visitor, walk_expr, walk_stmt},
};
use ruff_python_parser::parse_module;
use rustpython_compiler_core::SourceFile;

use crate::error::CodegenErrorType;
use crate::unparse::UnparseExpr;

#[derive(Debug, Clone)]
struct ContinuationMeta {
    decorator_kind: DecoratorKind,
    timeout_expr: String,
    guard_expr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoratorKind {
    Runner,
    Actor,
}

#[derive(Debug, Clone)]
struct AwaitStep {
    target_attr: String,
    job_type: String,
    args: String,
}

pub(crate) fn transform_mod(ast: Mod, source_file: &SourceFile) -> Result<Mod, CodegenErrorType> {
    match ast {
        Mod::Module(mut module) => {
            let mut new_body = Vec::new();
            for stmt in module.body {
                new_body.extend(transform_stmt(stmt, source_file)?);
            }
            module.body = new_body;
            Ok(Mod::Module(module))
        }
        other => Ok(other),
    }
}

fn transform_stmt(stmt: Stmt, source_file: &SourceFile) -> Result<Vec<Stmt>, CodegenErrorType> {
    match stmt {
        Stmt::ClassDef(mut class_def) => {
            let mut new_body = Vec::with_capacity(class_def.body.len());
            for inner in class_def.body.into_iter() {
                new_body.extend(transform_stmt(inner, source_file)?);
            }
            class_def.body = new_body;
            Ok(vec![Stmt::ClassDef(class_def)])
        }
        Stmt::FunctionDef(func_def) => transform_function(func_def, source_file),
        other => Ok(vec![other]),
    }
}

fn transform_function(
    func_def: StmtFunctionDef,
    source_file: &SourceFile,
) -> Result<Vec<Stmt>, CodegenErrorType> {
    if !func_def.is_async {
        return Ok(vec![Stmt::FunctionDef(func_def)]);
    }

    let meta = continuation_meta(&func_def.decorator_list, source_file)?;
    if meta.is_none() {
        if contains_await(&Stmt::FunctionDef(func_def.clone())) {
            return Err(CodegenErrorType::SyntaxError(
                "await is only allowed inside @runner.continuation/@actor.continuation".to_owned(),
            ));
        }
        return Ok(vec![Stmt::FunctionDef(func_def)]);
    }
    let meta = meta.unwrap();

    if func_def.decorator_list.len() != 1 {
        return Err(CodegenErrorType::SyntaxError(
            "continuation functions must not use extra decorators".to_owned(),
        ));
    }

    let (self_name, msg_name, params_str) = extract_params(&func_def.parameters)?;
    let (ctx_name, steps, return_expr) = parse_body(&func_def.body, source_file)?;

    if steps.is_empty() {
        return Err(CodegenErrorType::SyntaxError(
            "continuation function must contain at least one await".to_owned(),
        ));
    }

    let new_stmts = build_fsm_functions(
        func_def.name.as_str(),
        &params_str,
        self_name,
        msg_name,
        &ctx_name,
        &steps,
        &return_expr,
        &meta,
    )?;

    Ok(new_stmts)
}

fn continuation_meta(
    decorators: &[Decorator],
    source_file: &SourceFile,
) -> Result<Option<ContinuationMeta>, CodegenErrorType> {
    if decorators.is_empty() {
        return Ok(None);
    }
    let mut found = None;
    for deco in decorators {
        if let Some((kind, timeout_expr, guard_expr)) = parse_decorator(&deco.expression, source_file)? {
            if found.is_some() {
                return Err(CodegenErrorType::SyntaxError(
                    "multiple continuation decorators are not allowed".to_owned(),
                ));
            }
            found = Some(ContinuationMeta {
                decorator_kind: kind,
                timeout_expr,
                guard_expr,
            });
        } else {
            return Err(CodegenErrorType::SyntaxError(
                "continuation functions must only use @runner.continuation/@actor.continuation"
                    .to_owned(),
            ));
        }
    }
    Ok(found)
}

fn parse_decorator(
    expr: &Expr,
    source_file: &SourceFile,
) -> Result<Option<(DecoratorKind, String, String)>, CodegenErrorType> {
    let (kind, call) = match expr {
        Expr::Attribute(attr) => {
            if attr.attr.as_str() != "continuation" {
                return Ok(None);
            }
            let kind = decorator_base_kind(&attr.value)?;
            let timeout_expr = "0".to_owned();
            let guard_expr = "None".to_owned();
            return Ok(Some((kind, timeout_expr, guard_expr)));
        }
        Expr::Call(call) => {
            let Expr::Attribute(attr) = call.func.as_ref() else {
                return Ok(None);
            };
            if attr.attr.as_str() != "continuation" {
                return Ok(None);
            }
            let kind = decorator_base_kind(&attr.value)?;
            (kind, call)
        }
        _ => return Ok(None),
    };

    let mut timeout_expr = "0".to_owned();
    let mut guard_expr = "None".to_owned();
    for kw in &call.arguments.keywords {
        let Some(name) = &kw.arg else {
            return Err(CodegenErrorType::SyntaxError(
                "continuation decorator does not allow **kwargs".to_owned(),
            ));
        };
        let value = UnparseExpr::new(&kw.value, source_file).to_string();
        match name.as_str() {
            "timeout_blocks" => timeout_expr = value,
            "guard_unchanged" => guard_expr = value,
            _ => {
                return Err(CodegenErrorType::SyntaxError(format!(
                    "unsupported continuation decorator argument: {}",
                    name.as_str()
                )))
            }
        }
    }

    Ok(Some((kind, timeout_expr, guard_expr)))
}

fn decorator_base_kind(expr: &Expr) -> Result<DecoratorKind, CodegenErrorType> {
    match expr {
        Expr::Name(ExprName { id, .. }) if id.as_str() == "runner" => Ok(DecoratorKind::Runner),
        Expr::Name(ExprName { id, .. }) if id.as_str() == "actor" => Ok(DecoratorKind::Actor),
        _ => Err(CodegenErrorType::SyntaxError(
            "continuation decorator must be runner.continuation or actor.continuation".to_owned(),
        )),
    }
}

fn extract_params(
    params: &Parameters,
) -> Result<(&str, &str, String), CodegenErrorType> {
    let has_default = params
        .posonlyargs
        .iter()
        .chain(&params.args)
        .chain(&params.kwonlyargs)
        .any(|param| param.default.is_some());
    if !params.posonlyargs.is_empty()
        || !params.kwonlyargs.is_empty()
        || params.vararg.is_some()
        || params.kwarg.is_some()
        || has_default
    {
        return Err(CodegenErrorType::SyntaxError(
            "continuation functions must use simple (self, msg) parameters".to_owned(),
        ));
    }
    if params.args.len() != 2 {
        return Err(CodegenErrorType::SyntaxError(
            "continuation functions must use exactly (self, msg) parameters".to_owned(),
        ));
    }
    let self_name = params.args[0].name().as_str();
    let msg_name = params.args[1].name().as_str();
    Ok((self_name, msg_name, format!("{}, {}", self_name, msg_name)))
}

fn parse_body(
    body: &[Stmt],
    source_file: &SourceFile,
) -> Result<(String, Vec<AwaitStep>, String), CodegenErrorType> {
    if body.is_empty() {
        return Err(CodegenErrorType::SyntaxError(
            "continuation function body is empty".to_owned(),
        ));
    }

    let (ctx_name, start) = parse_capture(body)?;
    let mut steps = Vec::new();
    let mut return_expr = "None".to_owned();

    for stmt in &body[start..] {
        match stmt {
            Stmt::Assign(assign) => {
                let step = parse_await_assign(assign, &ctx_name, source_file)?;
                steps.push(step);
            }
            Stmt::Return(ret) => {
                return_expr = match &ret.value {
                    Some(expr) => UnparseExpr::new(expr, source_file).to_string(),
                    None => "None".to_owned(),
                };
            }
            _ => {
                return Err(CodegenErrorType::SyntaxError(
                    "only ctx.<name> = await runner.* and return are supported".to_owned(),
                ))
            }
        }
    }

    Ok((ctx_name, steps, return_expr))
}

fn parse_capture(body: &[Stmt]) -> Result<(String, usize), CodegenErrorType> {
    let mut start = 0;
    if let Some(Stmt::Expr(StmtExpr { value, .. })) = body.first() {
        if matches!(value.as_ref(), Expr::StringLiteral(_)) {
            start = 1;
        }
    }
    let first = body
        .get(start)
        .ok_or_else(|| CodegenErrorType::SyntaxError("empty body".to_owned()))?;
    let Stmt::Assign(StmtAssign { targets, value, .. }) = first else {
        return Err(CodegenErrorType::SyntaxError(
            "first statement must be ctx = capture()".to_owned(),
        ));
    };
    if targets.len() != 1 {
        return Err(CodegenErrorType::SyntaxError(
            "capture assignment must have one target".to_owned(),
        ));
    }
    let Expr::Name(ExprName { id, .. }) = &targets[0] else {
        return Err(CodegenErrorType::SyntaxError(
            "capture assignment target must be a name".to_owned(),
        ));
    };
    if !is_capture_call(value) {
        return Err(CodegenErrorType::SyntaxError(
            "first statement must be ctx = capture()".to_owned(),
        ));
    }
    Ok((id.to_string(), start + 1))
}

fn is_capture_call(expr: &Expr) -> bool {
    let Expr::Call(ExprCall { func, .. }) = expr else {
        return false;
    };
    match func.as_ref() {
        Expr::Name(ExprName { id, .. }) => id.as_str() == "capture",
        Expr::Attribute(ExprAttribute { value, attr, .. }) => {
            if attr.as_str() != "capture" {
                return false;
            }
            matches!(value.as_ref(), Expr::Name(ExprName { id, .. }) if id.as_str() == "pvm_sdk")
        }
        _ => false,
    }
}

fn parse_await_assign(
    assign: &StmtAssign,
    ctx_name: &str,
    source_file: &SourceFile,
) -> Result<AwaitStep, CodegenErrorType> {
    if assign.targets.len() != 1 {
        return Err(CodegenErrorType::SyntaxError(
            "await assignment must have one target".to_owned(),
        ));
    }
    let Expr::Attribute(attr) = &assign.targets[0] else {
        return Err(CodegenErrorType::SyntaxError(
            "await assignment target must be ctx.<name>".to_owned(),
        ));
    };
    let Expr::Name(ExprName { id, .. }) = attr.value.as_ref() else {
        return Err(CodegenErrorType::SyntaxError(
            "await assignment target must be ctx.<name>".to_owned(),
        ));
    };
    if id.as_str() != ctx_name {
        return Err(CodegenErrorType::SyntaxError(
            "await assignment target must be ctx.<name>".to_owned(),
        ));
    }
    let target_attr = attr.attr.as_str().to_owned();

    let Expr::Await(await_expr) = assign.value.as_ref() else {
        return Err(CodegenErrorType::SyntaxError(
            "await assignment must await runner.*".to_owned(),
        ));
    };
    let Expr::Call(call) = await_expr.value.as_ref() else {
        return Err(CodegenErrorType::SyntaxError(
            "await must call runner.*".to_owned(),
        ));
    };
    let Expr::Attribute(func_attr) = call.func.as_ref() else {
        return Err(CodegenErrorType::SyntaxError(
            "await must call runner.*".to_owned(),
        ));
    };
    let Expr::Name(ExprName { id, .. }) = func_attr.value.as_ref() else {
        return Err(CodegenErrorType::SyntaxError(
            "await must call runner.*".to_owned(),
        ));
    };
    if id.as_str() != "runner" {
        return Err(CodegenErrorType::SyntaxError(
            "await must call runner.*".to_owned(),
        ));
    }
    let job_type = func_attr.attr.as_str().to_owned();
    let args = format_call_args(&call.arguments, source_file)?;

    Ok(AwaitStep {
        target_attr,
        job_type,
        args,
    })
}

fn format_call_args(
    args: &Arguments,
    source_file: &SourceFile,
) -> Result<String, CodegenErrorType> {
    let mut parts = Vec::new();
    for arg in &args.args {
        parts.push(UnparseExpr::new(arg, source_file).to_string());
    }
    for kw in &args.keywords {
        let Some(name) = &kw.arg else {
            return Err(CodegenErrorType::SyntaxError(
                "runner calls do not allow **kwargs in continuation mode".to_owned(),
            ));
        };
        let value = UnparseExpr::new(&kw.value, source_file).to_string();
        parts.push(format!("{}={}", name.as_str(), value));
    }
    Ok(parts.join(", "))
}

fn build_fsm_functions(
    name: &str,
    params: &str,
    self_name: &str,
    msg_name: &str,
    ctx_name: &str,
    steps: &[AwaitStep],
    return_expr: &str,
    meta: &ContinuationMeta,
) -> Result<Vec<Stmt>, CodegenErrorType> {
    let first = &steps[0];
    let mut init_lines = Vec::new();
    init_lines.push(format!("def {}({}):", name, params));
    init_lines.push("    import pvm_sdk".to_owned());
    init_lines.push(format!(
        "    cid = pvm_sdk.continuation.new_cid({}, \"{}\")",
        self_name, name
    ));
    init_lines.push(format!("    {} = pvm_sdk.capture()", ctx_name));
    init_lines.push(format!(
        "    pvm_sdk.continuation.save_cont(cid, state=0, ctx={}, handler=\"{}__resume\", timeout_blocks={}, guard_unchanged={})",
        ctx_name, name, meta.timeout_expr, meta.guard_expr
    ));
    init_lines.push(format!(
        "    pvm_sdk.runner._send_job(\"{}\", cid, \"{}__resume\"{}{})",
        first.job_type,
        name,
        if first.args.is_empty() { "" } else { ", " },
        first.args
    ));
    init_lines.push("    return None".to_owned());

    let mut resume_lines = Vec::new();
    resume_lines.push(format!("def {}__resume({}):", name, params));
    resume_lines.push("    import pvm_sdk".to_owned());
    resume_lines.push(format!("    cid = {}.get(\"cid\")", msg_name));
    resume_lines.push("    st = pvm_sdk.continuation.load_cont(cid)".to_owned());
    resume_lines.push(format!("    {} = st.get(\"ctx\")", ctx_name));

    for (idx, step) in steps.iter().enumerate() {
        let is_last = idx == steps.len() - 1;
        resume_lines.push(format!("    if st.get(\"state\") == {}:", idx));
        resume_lines.push(format!(
            "        {}.{} = {}.get(\"result\")",
            ctx_name, step.target_attr, msg_name
        ));
        if is_last {
            resume_lines.push("        pvm_sdk.continuation.delete_cont(cid)".to_owned());
            resume_lines.push(format!("        return {}", return_expr));
        } else {
            resume_lines.push(format!(
                "        pvm_sdk.continuation.save_cont(cid, state={}, ctx={}, handler=\"{}__resume\", timeout_blocks=st.get(\"timeout_blocks\"), guard_unchanged=st.get(\"guard_unchanged\"))",
                idx + 1,
                ctx_name,
                name
            ));
            let next_step = &steps[idx + 1];
            resume_lines.push(format!(
                "        pvm_sdk.runner._send_job(\"{}\", cid, \"{}__resume\"{}{})",
                next_step.job_type,
                name,
                if next_step.args.is_empty() { "" } else { ", " },
                next_step.args
            ));
            resume_lines.push("        return None".to_owned());
        }
    }
    resume_lines.push("    return None".to_owned());

    let code = format!(
        "{}\n\n{}",
        init_lines.join("\n"),
        resume_lines.join("\n")
    );

    let parsed = parse_module(&code).map_err(|err| {
        CodegenErrorType::SyntaxError(format!("pvm fsm transform failed: {}", err.error))
    })?;
    let module = parsed.into_syntax();
    Ok(module.body)
}

fn contains_await(stmt: &Stmt) -> bool {
    let mut finder = AwaitFinder { found: false };
    finder.visit_stmt(stmt);
    finder.found
}

struct AwaitFinder {
    found: bool,
}

impl Visitor<'_> for AwaitFinder {
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr, Expr::Await(_)) {
            self.found = true;
            return;
        }
        walk_expr(self, expr);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        if self.found {
            return;
        }
        walk_stmt(self, stmt);
    }
}
