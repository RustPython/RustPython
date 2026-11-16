//! WIP ownership-disposal prototype, deliberately test-only.
//!
//! This is not wired into parser/compiler/VM entry points. Production integration
//! must retain ownership across early errors, transfers and subtree replacements.
//! Semantic Transformer skips runtime AST metadata and recursive ConstantValue
//! payloads, so those ownership edges are detached explicitly before walking.
//! No configured Python recursion limit is treated as a native-drop guarantee.
use ast::visitor::transformer::{self, Transformer};
use ruff_python_ast as ast;
use std::cell::RefCell;

enum Node {
    Expr(ast::Expr),
    Stmt(ast::Stmt),
    Pattern(ast::Pattern),
    Interpolation(ast::InterpolatedStringElement),
    TypeParam(ast::TypeParam),
    ExceptHandler(ast::ExceptHandler),
    Constant(ast::ConstantValue),
}
struct Drain(RefCell<Vec<Node>>);
impl Drain {
    fn push(&self, node: Node) {
        self.0.borrow_mut().push(node);
    }
    fn finish(&self) {
        loop {
            let next = self.0.borrow_mut().pop();
            let Some(mut node) = next else { break };
            match &mut node {
                Node::Expr(n) => {
                    self.expr_runtime(n);
                    transformer::walk_expr(self, n);
                }
                Node::Stmt(n) => {
                    self.stmt_runtime(n);
                    transformer::walk_stmt(self, n);
                }
                Node::Pattern(n) => {
                    self.pattern_runtime(n);
                    transformer::walk_pattern(self, n);
                }
                Node::Interpolation(n) => {
                    self.interpolation_runtime(n);
                    transformer::walk_interpolated_string_element(self, n);
                }
                Node::TypeParam(n) => self.visit_type_param(n),
                Node::ExceptHandler(n) => self.visit_except_handler(n),
                Node::Constant(n) => match n {
                    ast::ConstantValue::Tuple(xs) | ast::ConstantValue::Frozenset(xs) => {
                        for x in std::mem::take(xs) {
                            self.push(Node::Constant(x));
                        }
                    }
                    ast::ConstantValue::None
                    | ast::ConstantValue::Boolean(_)
                    | ast::ConstantValue::Str(_)
                    | ast::ConstantValue::Bytes(_)
                    | ast::ConstantValue::Integer(_)
                    | ast::ConstantValue::Float(_)
                    | ast::ConstantValue::Complex { .. }
                    | ast::ConstantValue::Ellipsis => {}
                },
            }
        }
    }
    fn expr_runtime(&self, node: &mut ast::Expr) {
        match node {
            ast::Expr::BoolOp(n) => {
                if let Some(xs) = n.runtime_values.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::Dict(n) => {
                if let Some(xs) = n.runtime_values.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::Set(n) => {
                if let Some(xs) = n.runtime_elts.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::Compare(n) => {
                if let Some(xs) = n.runtime_comparators.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::FString(n) => {
                if let Some(xs) = n.runtime_joined_str.take() {
                    for x in xs.into_iter() {
                        self.push(Node::Expr(x));
                    }
                }
                if let Some(xs) = n.runtime_values.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::TString(n) => {
                if let Some(xs) = n.runtime_template_str.take() {
                    for x in xs.into_iter() {
                        self.push(Node::Expr(x));
                    }
                }
                if let Some(xs) = n.runtime_values.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::List(n) => {
                if let Some(xs) = n.runtime_elts.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::Tuple(n) => {
                if let Some(xs) = n.runtime_elts.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Expr::Constant(n) => self.push(Node::Constant(std::mem::replace(
                &mut n.value,
                ast::ConstantValue::None,
            ))),
            _ => {}
        }
    }
    fn stmt_runtime(&self, node: &mut ast::Stmt) {
        match node {
            ast::Stmt::FunctionDef(n) => {
                if let Some(xs) = n.runtime_decorator_list.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            ast::Stmt::ClassDef(n) => {
                if let Some(xs) = n.runtime_decorator_list.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            ast::Stmt::Delete(n) => {
                if let Some(xs) = n.runtime_targets.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Stmt::Assign(n) => {
                if let Some(xs) = n.runtime_targets.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
            }
            ast::Stmt::For(n) => {
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
                if let Some(xs) = n.runtime_orelse.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            ast::Stmt::While(n) => {
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
                if let Some(xs) = n.runtime_orelse.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            ast::Stmt::If(n) => {
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            ast::Stmt::With(n) => {
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            ast::Stmt::Try(n) => {
                if let Some(xs) = n.runtime_body.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
                if let Some(xs) = n.runtime_handlers.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::ExceptHandler(x));
                    }
                }
                if let Some(xs) = n.runtime_orelse.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
                if let Some(xs) = n.runtime_finalbody.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Stmt(x));
                    }
                }
            }
            _ => {}
        }
    }
    fn pattern_runtime(&self, node: &mut ast::Pattern) {
        match node {
            ast::Pattern::MatchSequence(n) => {
                if let Some(xs) = n.runtime_patterns.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Pattern(x));
                    }
                }
            }
            ast::Pattern::MatchMapping(n) => {
                if let Some(xs) = n.runtime_keys.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Expr(x));
                    }
                }
                if let Some(xs) = n.runtime_patterns.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Pattern(x));
                    }
                }
            }
            ast::Pattern::MatchClass(n) => {
                if let Some(xs) = n.runtime_patterns.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Pattern(x));
                    }
                }
                if let Some(xs) = n.runtime_kwd_patterns.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Pattern(x));
                    }
                }
            }
            ast::Pattern::MatchOr(n) => {
                if let Some(xs) = n.runtime_patterns.take() {
                    for x in xs.into_iter().flatten() {
                        self.push(Node::Pattern(x));
                    }
                }
            }
            _ => {}
        }
    }
    fn interpolation_runtime(&self, node: &mut ast::InterpolatedStringElement) {
        if let ast::InterpolatedStringElement::Interpolation(n) = node {
            if let Some(x) = n.runtime_str.take() {
                self.push(Node::Constant(x));
            }
            if let Some(x) = n.runtime_interpolation_format_spec.take() {
                self.push(Node::Expr(*x));
            }
            if let Some(x) = n.runtime_formatted_value_format_spec.take() {
                self.push(Node::Expr(*x));
            }
        }
    }
}
impl Transformer for Drain {
    fn visit_expr(&self, n: &mut ast::Expr) {
        self.push(Node::Expr(std::mem::replace(
            n,
            ast::Expr::NoneLiteral(Default::default()),
        )));
    }
    fn visit_stmt(&self, n: &mut ast::Stmt) {
        self.push(Node::Stmt(std::mem::replace(
            n,
            ast::Stmt::Pass(ast::StmtPass {
                node_index: Default::default(),
                range: Default::default(),
            }),
        )));
    }
    fn visit_pattern(&self, n: &mut ast::Pattern) {
        self.push(Node::Pattern(std::mem::replace(
            n,
            ast::Pattern::MatchAs(ast::PatternMatchAs {
                node_index: Default::default(),
                range: Default::default(),
                pattern: None,
                name: None,
            }),
        )));
    }
    fn visit_interpolated_string_element(&self, n: &mut ast::InterpolatedStringElement) {
        self.push(Node::Interpolation(std::mem::replace(
            n,
            ast::InterpolatedStringElement::Literal(ast::InterpolatedStringLiteralElement {
                node_index: Default::default(),
                range: Default::default(),
                value: "".into(),
            }),
        )));
    }
    fn visit_elif_else_clause(&self, n: &mut ast::ElifElseClause) {
        if let Some(xs) = n.runtime_body.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Stmt(x));
            }
        }
        if let Some(xs) = n.runtime_orelse.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Stmt(x));
            }
        }
        transformer::walk_elif_else_clause(self, n);
    }
    fn visit_comprehension(&self, n: &mut ast::Comprehension) {
        if let Some(xs) = n.runtime_ifs.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Expr(x));
            }
        }
        transformer::walk_comprehension(self, n);
    }
    fn visit_parameters(&self, n: &mut ast::Parameters) {
        if let Some(xs) = n.runtime_defaults.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Expr(x));
            }
        }
        transformer::walk_parameters(self, n);
    }
    fn visit_arguments(&self, n: &mut ast::Arguments) {
        if let Some(xs) = n.runtime_args.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Expr(x));
            }
        }
        if let Some(xs) = n.runtime_bases.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Expr(x));
            }
        }
        transformer::walk_arguments(self, n);
    }
    fn visit_type_params(&self, n: &mut ast::TypeParams) {
        if let Some(xs) = n.runtime_type_params.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::TypeParam(x));
            }
        }
        transformer::walk_type_params(self, n);
    }
    fn visit_match_case(&self, n: &mut ast::MatchCase) {
        if let Some(xs) = n.runtime_body.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Stmt(x));
            }
        }
        transformer::walk_match_case(self, n);
    }
    fn visit_except_handler(&self, node: &mut ast::ExceptHandler) {
        let ast::ExceptHandler::ExceptHandler(n) = node;
        if let Some(xs) = n.runtime_body.take() {
            for x in xs.into_iter().flatten() {
                self.push(Node::Stmt(x));
            }
        }
        transformer::walk_except_handler(self, node);
    }
}
fn drain(mut tree: ast::Mod) {
    let d = Drain(RefCell::new(Vec::new()));
    match &mut tree {
        ast::Mod::Module(n) => {
            if let Some(xs) = n.runtime_body.take() {
                for x in xs.into_iter().flatten() {
                    d.push(Node::Stmt(x));
                }
            }
            d.visit_body(&mut n.body)
        }
        ast::Mod::Expression(n) => d.visit_expr(&mut n.body),
    }
    d.finish();
}

#[test]
fn deep_parser_trees() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            for (source, valid) in [
                ("-".repeat(100000) + "1", true),
                ("x".to_owned() + &".x".repeat(100000), true),
                ("-".repeat(100000), false),
            ] {
                let parsed = ruff_python_parser::parse_unchecked(
                    &source,
                    ruff_python_parser::Mode::Module.into(),
                );
                assert_eq!(parsed.errors().is_empty(), valid);
                drain(parsed.into_syntax());
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn deep_runtime_and_constant_edges() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut value = ast::ConstantValue::None;
            for i in 0..100000 {
                value = if i % 2 == 0 {
                    ast::ConstantValue::Tuple(vec![value])
                } else {
                    ast::ConstantValue::Frozenset(vec![value])
                };
            }
            let mut expr = ast::Expr::Constant(ast::ExprConstant {
                node_index: Default::default(),
                range: Default::default(),
                value,
                kind: None,
                invalid_type: None,
            });
            for _ in 0..100000 {
                expr = ast::Expr::List(ast::ExprList {
                    node_index: Default::default(),
                    range: Default::default(),
                    elts: Vec::new(),
                    ctx: ast::ExprContext::Load,
                    runtime_elts: Some(vec![None, Some(expr)]),
                });
            }
            let d = Drain(RefCell::new(vec![Node::Expr(expr)]));
            d.finish();
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn deep_format_spec_edges() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut child =
                ast::InterpolatedStringElement::Literal(ast::InterpolatedStringLiteralElement {
                    node_index: Default::default(),
                    range: Default::default(),
                    value: "x".into(),
                });
            for _ in 0..100000 {
                child = ast::InterpolatedStringElement::Interpolation(ast::InterpolatedElement {
                    node_index: Default::default(),
                    range: Default::default(),
                    expression: Box::new(ast::Expr::NoneLiteral(Default::default())),
                    debug_text: None,
                    conversion: ast::ConversionFlag::None,
                    format_spec: Some(Box::new(ast::InterpolatedStringFormatSpec {
                        node_index: Default::default(),
                        range: Default::default(),
                        elements: vec![child].into(),
                    })),
                    runtime_str: None,
                    runtime_interpolation_format_spec: None,
                    runtime_formatted_value_format_spec: None,
                });
            }
            let d = Drain(RefCell::new(vec![Node::Interpolation(child)]));
            d.finish();
        })
        .unwrap()
        .join()
        .unwrap();
}
