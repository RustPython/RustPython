use rustpython_ast::{Expr, ExprContext, ExprKind};

pub fn set_context(expr: Expr, ctx: ExprContext) -> Expr {
    match expr.node {
        ExprKind::Name { id, .. } => Expr::new(expr.start, expr.end, ExprKind::Name { id, ctx }),
        ExprKind::Tuple { elts, .. } => Expr::new(
            expr.start,
            expr.end,
            ExprKind::Tuple {
                elts: elts.into_iter().map(|elt| set_context(elt, ctx)).collect(),
                ctx,
            },
        ),
        ExprKind::List { elts, .. } => Expr::new(
            expr.start,
            expr.end,
            ExprKind::List {
                elts: elts.into_iter().map(|elt| set_context(elt, ctx)).collect(),
                ctx,
            },
        ),
        ExprKind::Attribute { value, attr, .. } => Expr::new(
            expr.start,
            expr.end,
            ExprKind::Attribute { value, attr, ctx },
        ),
        ExprKind::Subscript { value, slice, .. } => Expr::new(
            expr.start,
            expr.end,
            ExprKind::Subscript { value, slice, ctx },
        ),
        ExprKind::Starred { value, .. } => Expr::new(
            expr.start,
            expr.end,
            ExprKind::Starred {
                value: Box::new(set_context(*value, ctx)),
                ctx,
            },
        ),
        _ => expr,
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse_program;

    #[test]
    fn test_assign_name() {
        let source = String::from("x = (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_tuple() {
        let source = String::from("(x, y) = (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_list() {
        let source = String::from("[x, y] = (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_attribute() {
        let source = String::from("x.y = (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_subscript() {
        let source = String::from("x[y] = (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_starred() {
        let source = String::from("(x, *y) = (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_for() {
        let source = String::from("for x in (1, 2, 3): pass");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_list_comp() {
        let source = String::from("x = [y for y in (1, 2, 3)]");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_set_comp() {
        let source = String::from("x = {y for y in (1, 2, 3)}");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_with() {
        let source = String::from("with 1 as x: pass");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_assign_named_expr() {
        let source = String::from("if x:= 1: pass");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_ann_assign_name() {
        let source = String::from("x: int = 1");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_aug_assign_name() {
        let source = String::from("x += 1");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_aug_assign_attribute() {
        let source = String::from("x.y += (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_aug_assign_subscript() {
        let source = String::from("x[y] += (1, 2, 3)");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_del_name() {
        let source = String::from("del x");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_del_attribute() {
        let source = String::from("del x.y");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_del_subscript() {
        let source = String::from("del x[y]");
        let parse_ast = parse_program(&source, "<test>").unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }
}
